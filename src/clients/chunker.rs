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

use std::pin::Pin;

use async_trait::async_trait;
use axum::http::HeaderMap;
use futures::{Future, StreamExt, TryStreamExt};
use ginepro::LoadBalancedChannel;
use tonic::{Code, Request, Response, Status, Streaming};

use super::{
    BoxStream, Client, Error, create_grpc_client, errors::grpc_to_http_code,
    grpc_request_with_headers, otel_grpc::OtelGrpcService,
};
use crate::{
    config::ServiceConfig,
    health::{HealthCheckResult, HealthStatus},
    pb::{
        caikit::runtime::chunkers::{
            BidiStreamingChunkerTokenizationTaskRequest, ChunkerTokenizationTaskRequest,
            chunkers_service_client::ChunkersServiceClient,
        },
        caikit_data_model::nlp::{ChunkerTokenizationStreamResult, TokenizationResults},
        grpc::health::v1::{HealthCheckRequest, health_client::HealthClient},
    },
};

const DEFAULT_PORT: u16 = 8085;
pub const MODEL_ID_HEADER_NAME: &str = "mm-model-id";
/// Default chunker that returns span for entire text
pub const DEFAULT_CHUNKER_ID: &str = "whole_doc_chunker";

type StreamingTokenizationResult =
    Result<Response<Streaming<ChunkerTokenizationStreamResult>>, Status>;

#[derive(Clone)]
pub struct ChunkerClient {
    client: ChunkersServiceClient<OtelGrpcService<LoadBalancedChannel>>,
    health_client: HealthClient<OtelGrpcService<LoadBalancedChannel>>,
}

impl ChunkerClient {
    pub async fn new(config: &ServiceConfig) -> Self {
        let client = create_grpc_client(DEFAULT_PORT, config, ChunkersServiceClient::new).await;
        let health_client = create_grpc_client(DEFAULT_PORT, config, HealthClient::new).await;
        Self {
            client,
            health_client,
        }
    }

    pub async fn tokenization_task_predict(
        &self,
        model_id: &str,
        request: ChunkerTokenizationTaskRequest,
    ) -> Result<TokenizationResults, Error> {
        let mut client = self.client.clone();
        let request = request_with_headers(request, model_id);
        let response = client.chunker_tokenization_task_predict(request).await?;
        Ok(response.into_inner())
    }

    pub async fn bidi_streaming_tokenization_task_predict(
        &self,
        model_id: &str,
        request_stream: BoxStream<BidiStreamingChunkerTokenizationTaskRequest>,
    ) -> Result<BoxStream<Result<ChunkerTokenizationStreamResult, Error>>, Error> {
        let mut client = self.client.clone();
        let request = request_with_headers(request_stream, model_id);
        // NOTE: this is an ugly workaround to avoid bogus higher-ranked lifetime errors.
        // https://github.com/rust-lang/rust/issues/110338
        let response_stream_fut: Pin<Box<dyn Future<Output = StreamingTokenizationResult> + Send>> =
            Box::pin(client.bidi_streaming_chunker_tokenization_task_predict(request));
        let response_stream = response_stream_fut.await?;
        Ok(response_stream.into_inner().map_err(Into::into).boxed())
    }
}

#[async_trait]
impl Client for ChunkerClient {
    fn name(&self) -> &str {
        "chunker"
    }

    async fn health(&self, _headers: HeaderMap) -> HealthCheckResult {
        let mut client = self.health_client.clone();
        let response = client
            .check(HealthCheckRequest { service: "".into() })
            .await;
        let code = match response {
            Ok(_) => Code::Ok,
            Err(status) if matches!(status.code(), Code::InvalidArgument | Code::NotFound) => {
                Code::Ok
            }
            Err(status) => status.code(),
        };
        let status = if matches!(code, Code::Ok) {
            HealthStatus::Healthy
        } else {
            HealthStatus::Unhealthy
        };
        HealthCheckResult {
            status,
            code: grpc_to_http_code(code),
            reason: None,
        }
    }
}

/// Turns a chunker client gRPC request body of type `T` into a `tonic::Request<T>` with headers.
/// Adds the provided `model_id` as a header as well as injects `traceparent` from the current span.
fn request_with_headers<T>(request: T, model_id: &str) -> Request<T> {
    let mut request = grpc_request_with_headers(request, HeaderMap::new());
    request
        .metadata_mut()
        .insert(MODEL_ID_HEADER_NAME, model_id.parse().unwrap());
    request
}
