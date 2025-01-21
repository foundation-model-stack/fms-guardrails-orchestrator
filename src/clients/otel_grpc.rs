/*
 Copyright FMS Guardrails Orchestrator Authors

 Licensed under the Apache License, Version 2.0 (the "License");
 you may not use this file except in compliance with the License.
 You may obtain a copy of the License at

     http://www.apache.org/licenses/LICENSE-2.0

 Unless required by applicable law or agreed to in writing, software
 distributed under the License is distributed on an "AS IS" BASIS,
 WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 See the License for the specific language governing permissions and
 limitations under the License.

*/

use std::{
    error::Error,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use http::{Request, Response, StatusCode};
use pin_project_lite::pin_project;
use tokio::time::Instant;
use tonic::client::GrpcService;
use tower::Layer;
use tracing::{error, info, info_span, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::utils::trace::{current_trace_id, with_traceparent_header};

// Adapted from https://github.com/davidB/tracing-opentelemetry-instrumentation-sdk/tree/main/tonic-tracing-opentelemetry
/// Layer for grpc (tonic client):
///
/// - Propagate `OpenTelemetry` context (`trace_id`,...)
/// - Create a Span for `OpenTelemetry` on call
///
/// `OpenTelemetry` context are extracted from tracing's span.
#[derive(Default, Debug, Clone)]
pub struct OtelGrpcLayer;

impl<S> Layer<S> for OtelGrpcLayer {
    /// The wrapped service
    type Service = OtelGrpcService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        OtelGrpcService { inner }
    }
}

#[derive(Debug, Clone)]
pub struct OtelGrpcService<S> {
    inner: S,
}

/// Construct info span on grpc client request
fn make_span_from_request<B>(req: &http::Request<B>) -> tracing::Span {
    info_span!(
        "client gRPC request",
        request_method = req.method().to_string(),
        request_path = req.uri().path().to_string(),
    )
}

/// Log metrics based on response
fn log_on_response_or_error<B, E>(latency: u128, response: &Result<http::Response<B>, E>)
where
    E: Error,
{
    match response {
        Ok(response) => {
            info!(
                trace_id = %current_trace_id(),
                status = %response.status(),
                duration_ms = %latency,
                monotonic_counter.client_response_count = 1,
                histogram.client_request_duration = latency as u64,
                "finished processing request",
            );
            if response.status().is_server_error() {
                // On every server error (HTTP 5xx) response
                info!(monotonic_counter.client_5xx_response_count = 1);
            } else if response.status().is_client_error() {
                // On every client error (HTTP 4xx) response
                info!(monotonic_counter.client_4xx_response_count = 1);
            } else if response.status().is_success() {
                // On every successful (HTTP 2xx) response
                info!(monotonic_counter.client_success_response_count = 1);
            } else {
                error!(
                    "unexpected gRPC client response status code: {}",
                    response.status().as_u16()
                );
            }
        }
        Err(err) => {
            error!(trace_id = %current_trace_id(), latency_ms = %latency, "failure handling request: {}", err.to_string(),);

            info!(
                monotonic_counter.client_request_failure_count = 1,
                monotonic_counter.client_5xx_response_count = 1,
                latency_ms = %latency,
                status_code = %StatusCode::INTERNAL_SERVER_ERROR,
                ?err
            );
        }
    }
}

impl<S, B, B2> GrpcService<B> for OtelGrpcService<S>
where
    S: GrpcService<B, ResponseBody = B2> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Error + 'static,
    B: Send + 'static,
    B2: http_body::Body,
{
    type ResponseBody = B2;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        // This is necessary because tonic internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let span = make_span_from_request(&req);
        let ctx = span.context();
        let _headers = with_traceparent_header(&ctx, req.headers().clone());
        let _guard = span.enter();
        info!(
            trace_id = %current_trace_id(),
            method = %req.method(),
            path = %req.uri().path(),
            monotonic_counter.incoming_request_count = 1,
            "started processing request",
        );
        let future = {
            let _enter = span.enter();
            self.inner.call(req)
        };
        ResponseFuture {
            inner: future,
            span: span.clone(),
        }
    }
}

pin_project! {
    /// Response future for [`Trace`].
    ///
    /// [`Trace`]: super::Trace
    pub struct ResponseFuture<F> {
        #[pin]
        pub(crate) inner: F,
        pub(crate) span: Span,
    }
}

impl<Fut, ResBody, E> Future for ResponseFuture<Fut>
where
    Fut: Future<Output = Result<Response<ResBody>, E>>,
    E: std::error::Error + 'static,
{
    type Output = Result<Response<ResBody>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _guard = this.span.enter();
        let start_time = Instant::now();
        let result = futures_util::ready!(this.inner.poll(cx));
        let request_duration_ms = Instant::now()
            .checked_duration_since(start_time)
            .unwrap()
            .as_millis();
        log_on_response_or_error(request_duration_ms, &result);
        Poll::Ready(result)
    }
}
