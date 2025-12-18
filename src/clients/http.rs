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

use std::{fmt::Debug, ops::Deref, time::Duration};

use http::header::HeaderValue;
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{
    HeaderMap, Method, Request, StatusCode,
    body::{Bytes, Incoming},
};
use hyper_rustls::HttpsConnector;
use hyper_timeout::TimeoutConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use serde::{Serialize, de::DeserializeOwned};
use tower::{Service, timeout::Timeout};
use tower_http::{
    classify::{
        NeverClassifyEos, ServerErrorsAsFailures, ServerErrorsFailureClass, SharedClassifier,
    },
    trace::{
        DefaultOnBodyChunk, HttpMakeClassifier, MakeSpan, OnEos, OnFailure, OnRequest, OnResponse,
        Trace, TraceLayer,
    },
};
use tracing::{Span, error, info, info_span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use url::Url;

use super::{Client, Error};
use crate::{
    health::{HealthCheckResult, HealthStatus, OptionalHealthCheckResponseBody},
    utils::{AsUriExt, trace},
};

pub const JSON_CONTENT_TYPE: HeaderValue = HeaderValue::from_static("application/json");

/// Any type that implements Debug and Serialize can be used as a request body
pub trait RequestBody: Debug + Serialize {}

impl<T> RequestBody for T where T: Debug + Serialize {}

/// Any type that implements Debug and Deserialize can be used as a response body
pub trait ResponseBody: Debug + DeserializeOwned {}

impl<T> ResponseBody for T where T: Debug + DeserializeOwned {}

/// HTTP response type, thin wrapper for `hyper::http::Response` with extra functionality.
pub struct Response(pub hyper::http::Response<BoxBody<Bytes, hyper::Error>>);

impl Response {
    /// Deserializes the response body as JSON into type `T`.
    pub async fn json<T: DeserializeOwned>(self) -> Result<T, Error> {
        let data = self
            .0
            .into_body()
            .collect()
            .await
            .expect("unexpected infallible error")
            .to_bytes();
        serde_json::from_slice::<T>(&data).map_err(|e| Error::Http {
            code: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("client response deserialization failed: {e}"),
        })
    }
}

impl Deref for Response {
    type Target = hyper::http::response::Response<BoxBody<Bytes, hyper::Error>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<TracedResponse> for Response {
    fn from(response: TracedResponse) -> Self {
        Response(response.map(|body| body.boxed()))
    }
}

pub type HttpClientInner = Trace<
    Timeout<
        hyper_util::client::legacy::Client<
            TimeoutConnector<HttpsConnector<HttpConnector>>,
            BoxBody<Bytes, hyper::Error>,
        >,
    >,
    SharedClassifier<ServerErrorsAsFailures>,
    ClientMakeSpan,
    ClientOnRequest,
    ClientOnResponse,
    DefaultOnBodyChunk,
    ClientOnEos,
    ClientOnFailure,
>;

/// A trait implemented by all clients that use HTTP for their inner client.
pub trait HttpClientExt: Client {
    fn inner(&self) -> &HttpClient;
}

/// An HTTP client wrapping an inner `hyper` HTTP client providing a higher-level API.
#[derive(Clone)]
pub struct HttpClient {
    base_url: Url,
    health_url: Url,
    api_token: Option<String>,
    inner: HttpClientInner,
}

impl HttpClient {
    pub fn new(base_url: Url, api_token: Option<String>, inner: HttpClientInner) -> Self {
        // Ensure base_url has trailing slash for proper path joining
        let mut base_url = base_url;
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let health_url = base_url.join("health").unwrap();
        Self {
            base_url,
            health_url,
            api_token,
            inner,
        }
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub fn endpoint(&self, path: &str) -> Url {
        let path = path.trim_start_matches('/');
        self.base_url.join(path).unwrap()
    }

    /// Injects the API token as a Bearer token in the Authorization header if configured and present in the environment.
    fn inject_api_token(&self, headers: &mut HeaderMap) -> Result<(), Error> {
        if let Some(token) = &self.api_token {
            headers.insert(
                http::header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", token)).map_err(|e| Error::Http {
                    code: StatusCode::INTERNAL_SERVER_ERROR,
                    message: format!("invalid authorization header: {e}"),
                })?,
            );
        }
        Ok(())
    }
    pub async fn get(
        &self,
        url: Url,
        headers: HeaderMap,
        body: impl RequestBody,
    ) -> Result<Response, Error> {
        self.send(url, Method::GET, headers, body).await
    }

    pub async fn post(
        &self,
        url: Url,
        headers: HeaderMap,
        body: impl RequestBody,
    ) -> Result<Response, Error> {
        self.send(url, Method::POST, headers, body).await
    }

    pub async fn send(
        &self,
        url: Url,
        method: Method,
        mut headers: HeaderMap,
        body: impl RequestBody,
    ) -> Result<Response, Error> {
        let ctx = Span::current().context();

        self.inject_api_token(&mut headers)?;
        let headers = trace::with_traceparent_header(&ctx, headers);

        let mut builder = hyper::http::request::Builder::new()
            .method(method)
            .uri(url.as_uri());
        match builder.headers_mut() {
            Some(headers_mut) => {
                headers_mut.extend(headers);
                let body =
                    Full::new(Bytes::from(serde_json::to_vec(&body).map_err(|e| {
                        Error::Http {
                            code: StatusCode::INTERNAL_SERVER_ERROR,
                            message: format!("client request serialization failed: {e}")
                        }
                    })?))
                        .map_err(|err| match err {});
                let request = builder
                    .body(body.boxed())
                    .map_err(|e| {
                        Error::Http {
                            code: StatusCode::INTERNAL_SERVER_ERROR,
                            message: format!("client request serialization failed: {e}")
                        }
                    })?;
                let response = match self
                    .inner
                    .clone()
                    .call(request)
                    .await {
                        Ok(response) => Ok(response.map_err(|e| {
                            Error::Http {
                                code: StatusCode::INTERNAL_SERVER_ERROR,
                                message: format!("sending client request failed: {e}")
                            }
                        }).into_inner()),
                        Err(e) => Err(Error::Http {
                            code: StatusCode::REQUEST_TIMEOUT,
                            message: format!("client request timeout: {e}"),
                        }),
                }?;
                Ok(response.into())
            }
            None => Err(builder.body(body).err().map_or_else(
                || panic!("unexpected request builder error - headers missing in builder but no errors found"),
                |e| Error::Http {
                    code: StatusCode::INTERNAL_SERVER_ERROR,
                    message: format!("client request creation failed: {e}"),
                }
            )),
        }
    }

    pub async fn health(&self, headers: HeaderMap) -> HealthCheckResult {
        let mut builder = Request::get(self.health_url.as_uri());
        *builder.headers_mut().unwrap() = headers;
        let req = builder.body(BoxBody::default()).unwrap();
        let res = self.inner.clone().call(req).await;
        match res {
            Ok(response) => {
                let response = Response::from(response);
                if response.status() == StatusCode::OK {
                    if let Ok(body) = response.json::<OptionalHealthCheckResponseBody>().await {
                        // If the service provided a body, we only anticipate a minimal health status and optional reason.
                        HealthCheckResult {
                            status: body.status.clone(),
                            code: StatusCode::OK,
                            reason: match body.status {
                                HealthStatus::Healthy => None,
                                _ => body.reason,
                            },
                        }
                    } else {
                        // If the service did not provide a body, we assume it is healthy.
                        HealthCheckResult {
                            status: HealthStatus::Healthy,
                            code: StatusCode::OK,
                            reason: None,
                        }
                    }
                } else {
                    HealthCheckResult {
                        // The most we can presume is that 5xx errors are likely indicating service issues, implying the service is unhealthy.
                        // and that 4xx errors are more likely indicating health check failures, i.e. due to configuration/implementation issues.
                        // Regardless we can't be certain, so the reason is also provided.
                        // TODO: We will likely circle back to re-evaluate this logic in the future
                        // when we know more about how the client health results will be used.
                        status: if response.status().as_u16() >= 500
                            && response.status().as_u16() < 600
                        {
                            HealthStatus::Unhealthy
                        } else if response.status().as_u16() >= 400
                            && response.status().as_u16() < 500
                        {
                            HealthStatus::Unknown
                        } else {
                            error!(
                                "unexpected http health check status code: {}",
                                response.status()
                            );
                            HealthStatus::Unknown
                        },
                        code: response.status(),
                        reason: response.status().canonical_reason().map(|v| v.to_string()),
                    }
                }
            }
            Err(e) => {
                error!("error checking health: {}", e);
                HealthCheckResult {
                    status: HealthStatus::Unknown,
                    code: StatusCode::INTERNAL_SERVER_ERROR,
                    reason: Some(e.to_string()),
                }
            }
        }
    }
}

pub type TracedResponse = hyper::Response<
    tower_http::trace::ResponseBody<
        Incoming,
        NeverClassifyEos<ServerErrorsFailureClass>,
        DefaultOnBodyChunk,
        ClientOnEos,
        ClientOnFailure,
    >,
>;

pub type HttpClientTraceLayer = TraceLayer<
    HttpMakeClassifier,
    ClientMakeSpan,
    ClientOnRequest,
    ClientOnResponse,
    DefaultOnBodyChunk, // no metrics currently per body chunk
    ClientOnEos,
    ClientOnFailure,
>;

pub fn http_trace_layer() -> HttpClientTraceLayer {
    TraceLayer::new_for_http()
        .make_span_with(ClientMakeSpan)
        .on_request(ClientOnRequest)
        .on_response(ClientOnResponse)
        .on_failure(ClientOnFailure)
        .on_eos(ClientOnEos)
}

#[derive(Debug, Clone)]
pub struct ClientMakeSpan;

impl MakeSpan<BoxBody<Bytes, hyper::Error>> for ClientMakeSpan {
    fn make_span(&mut self, request: &Request<BoxBody<Bytes, hyper::Error>>) -> Span {
        info_span!(
            "client HTTP request",
            request_method = request.method().to_string(),
            request_path = request.uri().path().to_string(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct ClientOnRequest;

impl OnRequest<BoxBody<Bytes, hyper::Error>> for ClientOnRequest {
    fn on_request(&mut self, request: &Request<BoxBody<Bytes, hyper::Error>>, span: &Span) {
        let _guard = span.enter();
        info!(
            trace_id = %trace::current_trace_id(),
            method = %request.method(),
            path = %request.uri().path(),
            monotonic_counter.incoming_request_count = 1,
            "started processing request",
        );
    }
}

#[derive(Debug, Clone)]
pub struct ClientOnResponse;

impl OnResponse<Incoming> for ClientOnResponse {
    fn on_response(self, response: &hyper::Response<Incoming>, latency: Duration, span: &Span) {
        let _guard = span.enter();
        // On every response
        info!(
            trace_id = %trace::current_trace_id(),
            status = %response.status(),
            duration_ms = %latency.as_millis(),
            monotonic_counter.client_response_count = 1,
            histogram.client_request_duration = latency.as_millis() as u64,
            "finished processing request"
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
                "unexpected HTTP client response status code: {}",
                response.status().as_u16()
            );
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClientOnFailure;

impl OnFailure<ServerErrorsFailureClass> for ClientOnFailure {
    fn on_failure(
        &mut self,
        failure_classification: ServerErrorsFailureClass,
        latency: Duration,
        span: &Span,
    ) {
        let _guard = span.enter();
        let trace_id = trace::current_trace_id();
        let latency_ms = latency.as_millis().to_string();

        let (status_code, error) = match failure_classification {
            ServerErrorsFailureClass::StatusCode(status_code) => {
                error!(
                    %trace_id,
                    %status_code,
                    latency_ms,
                    "failure handling request",
                );
                (Some(status_code), None)
            }
            ServerErrorsFailureClass::Error(error) => {
                error!(?trace_id, latency_ms, "failure handling request: {}", error,);
                (None, Some(error))
            }
        };

        info!(
            monotonic_counter.client_request_failure_count = 1,
            monotonic_counter.client_5xx_response_count = 1,
            latency_ms,
            ?status_code,
            ?error
        );
    }
}

#[derive(Debug, Clone)]
pub struct ClientOnEos;

impl OnEos for ClientOnEos {
    fn on_eos(self, _trailers: Option<&HeaderMap>, stream_duration: Duration, span: &Span) {
        let _guard = span.enter();
        info!(
            trace_id = %trace::current_trace_id(),
            duration_ms = stream_duration.as_millis(),
            monotonic_counter.client_stream_response_count = 1,
            histogram.client_stream_response_duration = stream_duration.as_millis() as u64,
            "end of stream",
        );
    }
}

/// Extracts a base url from a url including path segments.
pub fn extract_base_url(url: &Url) -> Option<Url> {
    let mut url = url.clone();
    match url.path_segments_mut() {
        Ok(mut path) => {
            path.clear();
        }
        Err(_) => {
            return None;
        }
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_base_url() {
        let url =
            Url::parse("https://example-detector.route.example.com/api/v1/text/contents").unwrap();
        let base_url = extract_base_url(&url);
        assert_eq!(
            Some(Url::parse("https://example-detector.route.example.com/").unwrap()),
            base_url
        );
        let health_url = base_url.map(|v| v.join("/health").unwrap());
        assert_eq!(
            Some(Url::parse("https://example-detector.route.example.com/health").unwrap()),
            health_url
        );
    }
}
