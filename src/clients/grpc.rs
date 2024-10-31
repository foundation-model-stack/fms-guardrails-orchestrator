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
use std::future::Future;

#[derive(Clone)]
pub struct GrpcClient<C> {
    pub service_name: String,
    pub inner: C,
    pub tracing_enabled: bool,
}

impl<C: Clone> GrpcClient<C> {
    pub fn new(service_name: String, inner: C, tracing_enabled: bool) -> Self {
        Self {
            service_name,
            inner,
            tracing_enabled,
        }
    }
}

/// Work around for compiler bug: https://github.com/rust-lang/rust/issues/96865
pub trait SendFuture: Future {
    fn send(self) -> impl Future<Output = Self::Output> + Send
    where
        Self: Sized + Send,
    {
        self
    }
}

impl<T: Future> SendFuture for T {}

#[macro_export]
macro_rules! grpc_call {
    ($client:expr, $request:expr, $method:expr) => {{
        use tokio::time::Instant;
        use tracing::{debug, info};

        use $crate::{tracing_utils::{current_trace_id, trace_context}};

        let mut client = $client.inner.clone();

        // For health clients
        if !$client.tracing_enabled {
            let response = $method(&mut client, $request).await?;
            return Ok(response.into_inner())
        }

        let trace_id = current_trace_id();
        let protocol = "gRPC";
        let service_name = $client.service_name.clone();
        let request_metadata = $request.metadata();

        info!(?trace_id,
            protocol,
            service_name,
            // method,
            ?request_metadata,
            "sending client request"
        );
        info!(
            monotonic_counter.client_request_count = 1,
            ?trace_id,
            protocol,
            service_name,
            // method,
            ?request_metadata,
        );
        debug!(outgoing_client_request = ?$request);

        let start_time = Instant::now();
        let client_result = $method(&mut client, $request).await;
        let request_duration_ms = Instant::now().checked_duration_since(start_time).unwrap().as_millis();

        let (result, response_status, response_metadata) = match client_result {
            Ok(response) => {
                let response_status = tonic::Code::Ok;
                let response_metadata = response.metadata().clone();

                info!(
                    ?trace_id,
                    protocol,
                    service_name,
                    // method,
                    request_duration_ms,
                    ?response_status,
                    ?response_metadata,
                    "client response received",
                );
                debug!(incoming_client_response = ?response);
                info!(
                    monotonic_counter.success_client_response_count = 1,
                    ?trace_id,
                    protocol,
                    service_name,
                    // method,
                    request_duration_ms,
                    ?response_status,
                    ?response_metadata,
                );

                (Ok(response.into_inner()), response_status, response_metadata)
            }
            Err(error) => {
                let response_status = error.code();
                let response_metadata = error.metadata().clone();
                let error_message = error.message();

                info!(?trace_id, ?response_status, "received client error: {}", error_message);

                info!(
                    monotonic_counter.client_error_response_count = 1,
                    ?trace_id,
                    protocol,
                    service_name,
                    // method,
                    request_duration_ms,
                    ?response_status,
                    ?response_metadata,
                );

                (Err(error), response_status, response_metadata)
            }
        };

        info!(
            monotonic_counter.client_response_count = 1,
            ?trace_id,
            protocol,
            service_name,
            // method,
            request_duration_ms,
            ?response_status,
            ?response_metadata,
        );
        info!(
            histogram.client_request_duration = request_duration_ms,
            ?trace_id,
            protocol,
            service_name,
            // method,
            request_duration_ms,
            ?response_status,
            ?response_metadata,
        );

        trace_context(&response_metadata.clone().into_headers());
        Ok(result?)
    }};
}

#[macro_export]
macro_rules! grpc_stream_call {
    ($client:expr, $request:expr, $method:expr) => {{
        use std::{future::Future, pin::Pin};

        use tokio::time::Instant;
        use tracing::info;

        use $crate::{
            clients::grpc::SendFuture,
            tracing_utils::{current_trace_id, trace_context},
        };

        let mut client = $client.inner.clone();

        // For health clients
        if !$client.tracing_enabled {
            let response = Box::pin($method(&mut client, $request)).send().await?;
            return Ok(response.into_inner().map_err(Into::into).boxed());
        }

        let trace_id = current_trace_id();
        let protocol = "gRPC";
        let service_name = $client.service_name.clone();
        let request_metadata = $request.metadata();

        info!(
            ?trace_id,
            protocol,
            service_name,
            // method,
            ?request_metadata,
            "sending client request"
        );
        info!(
            monotonic_counter.client_request_count = 1,
            ?trace_id,
            protocol,
            service_name,
            // method,
            ?request_metadata,
        );
        // debug!(outgoing_client_request = ?$request);

        let start_time = Instant::now();
        let client_result: Pin<
            Box<
                dyn Future<Output = Result<tonic::Response<tonic::Streaming<_>>, tonic::Status>>
                    + Send,
            >,
        > = Box::pin($method(&mut client, $request));
        let request_duration_ms = Instant::now()
            .checked_duration_since(start_time)
            .unwrap()
            .as_millis();

        let (result, response_status, response_metadata) = match client_result.await {
            Ok(response) => {
                let response_status = tonic::Code::Ok;
                let response_metadata = response.metadata().clone();

                info!(
                    ?trace_id,
                    protocol,
                    service_name,
                    // method,
                    request_duration_ms,
                    ?response_status,
                    ?response_metadata,
                    "client response received",
                );
                // debug!(incoming_client_response = ?response);
                info!(
                    monotonic_counter.success_client_response_count = 1,
                    ?trace_id,
                    protocol,
                    service_name,
                    // method,
                    request_duration_ms,
                    ?response_status,
                    ?response_metadata,
                );

                (
                    Ok(response.into_inner().map_err(Into::into)),
                    response_status,
                    response_metadata,
                )
            }
            Err(error) => {
                let response_status = error.code();
                let response_metadata = error.metadata().clone();
                let error_message = error.message();

                info!(
                    ?trace_id,
                    ?response_status,
                    "received client error: {}",
                    error_message
                );

                info!(
                    monotonic_counter.client_error_response_count = 1,
                    ?trace_id,
                    protocol,
                    service_name,
                    // method,
                    request_duration_ms,
                    ?response_status,
                    ?response_metadata,
                );

                (Err(error), response_status, response_metadata)
            }
        };

        info!(
            monotonic_counter.client_response_count = 1,
            ?trace_id,
            protocol,
            service_name,
            // method,
            request_duration_ms,
            ?response_status,
            ?response_metadata,
        );
        info!(
            histogram.client_request_duration = request_duration_ms,
            ?trace_id,
            protocol,
            service_name,
            // method,
            request_duration_ms,
            ?response_status,
            ?response_metadata,
        );

        trace_context(&response_metadata.clone().into_headers());
        match result {
            Ok(result) => Ok(result.boxed()),
            Err(error) => Err(error.into()),
        }
    }};
}
