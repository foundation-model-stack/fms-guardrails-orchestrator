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

use async_trait::async_trait;
use axum::http::HeaderMap;
use futures::{Stream, StreamExt, TryStreamExt};
use ginepro::LoadBalancedChannel;
use hyper::StatusCode;
use tonic::{Request, Response, Status, Streaming};
use tracing::instrument;

use super::{
    create_grpc_client, grpc::GrpcClient, grpc_request_with_headers, BoxStream, Client, Error,
};
use crate::pb::grpc::health::v1::HealthCheckResponse;
use crate::{
    config::ServiceConfig,
    grpc_call, grpc_stream_call,
    health::{HealthCheckResult, HealthStatus},
    pb::{
        caikit::runtime::chunkers::{
            chunkers_service_client::ChunkersServiceClient, chunkers_service_server,
            BidiStreamingChunkerTokenizationTaskRequest, ChunkerTokenizationTaskRequest,
        },
        caikit_data_model::nlp::{ChunkerTokenizationStreamResult, Token, TokenizationResults},
        grpc::health::v1::{health_client::HealthClient, health_server, HealthCheckRequest},
    },
};

const DEFAULT_PORT: u16 = 8085;
const MODEL_ID_HEADER_NAME: &str = "mm-model-id";
/// Default chunker that returns span for entire text
pub const DEFAULT_CHUNKER_ID: &str = "whole_doc_chunker";

type StreamingTokenizationResult =
    Result<Response<Streaming<ChunkerTokenizationStreamResult>>, Status>;

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct ChunkerClient {
    client: GrpcClient<ChunkersServiceClient<LoadBalancedChannel>>,
    health_client: GrpcClient<HealthClient<LoadBalancedChannel>>,
}

#[cfg_attr(test, faux::methods)]
impl ChunkerClient {
    pub async fn new(config: &ServiceConfig) -> Self {
        let client = create_grpc_client(
            chunkers_service_server::SERVICE_NAME,
            DEFAULT_PORT,
            config,
            ChunkersServiceClient::new,
            true,
        )
        .await;
        let health_client = create_grpc_client(
            health_server::SERVICE_NAME,
            DEFAULT_PORT,
            config,
            HealthClient::new,
            false,
        )
        .await;
        Self {
            client,
            health_client,
        }
    }

    #[instrument(skip_all, fields(model_id))]
    pub async fn tokenization_task_predict(
        &self,
        model_id: &str,
        request: ChunkerTokenizationTaskRequest,
    ) -> Result<TokenizationResults, Error> {
        let request = request_with_headers(request, model_id);
        grpc_call!(
            self.client,
            request,
            ChunkersServiceClient::chunker_tokenization_task_predict
        )
    }

    #[instrument(skip_all, fields(model_id))]
    pub async fn bidi_streaming_tokenization_task_predict(
        &self,
        model_id: &str,
        request_stream: BoxStream<BidiStreamingChunkerTokenizationTaskRequest>,
    ) -> Result<BoxStream<Result<ChunkerTokenizationStreamResult, Error>>, Error> {
        let request = request_with_headers(request_stream, model_id);
        // let mut client = self.client.inner.clone();
        // // NOTE: this is an ugly workaround to avoid bogus higher-ranked lifetime errors.
        // // https://github.com/rust-lang/rust/issues/110338
        // let response_stream_fut: Pin<Box<dyn Future<Output = StreamingTokenizationResult> + Send>> =
        //     Box::pin(client.bidi_streaming_chunker_tokenization_task_predict(request));
        // let response_stream = response_stream_fut.await?;
        // trace_context_from_grpc_response(&response_stream);
        // Ok(response_stream.into_inner().map_err(Into::into).boxed())
        grpc_stream_call!(
            self.client,
            request,
            ChunkersServiceClient::bidi_streaming_chunker_tokenization_task_predict
        )
    }
}

#[cfg_attr(test, faux::methods)]
#[async_trait]
impl Client for ChunkerClient {
    fn name(&self) -> &str {
        "chunker"
    }

    async fn health(&self) -> Result<HealthCheckResult, Error> {
        let request =
            grpc_request_with_headers(HealthCheckRequest { service: "".into() }, HeaderMap::new());
        let response: Result<HealthCheckResponse, Error> =
            async { grpc_call!(self.health_client, request, HealthClient::check) }.await;
        let code = match response {
            Ok(_) => StatusCode::OK,
            Err(error) => match error {
                Error::Grpc {
                    code: StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND,
                    ..
                } => StatusCode::OK,
                Error::Grpc { code, .. } => code,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
        };
        let status = if matches!(code, StatusCode::OK) {
            HealthStatus::Healthy
        } else {
            HealthStatus::Unhealthy
        };
        Ok(HealthCheckResult {
            status,
            code,
            reason: None,
        })
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

/// Unary tokenization result of the entire doc
#[instrument(skip_all)]
pub fn tokenize_whole_doc(request: ChunkerTokenizationTaskRequest) -> TokenizationResults {
    let codepoint_count = request.text.chars().count() as i64;
    TokenizationResults {
        results: vec![Token {
            start: 0,
            end: codepoint_count,
            text: request.text,
        }],
        token_count: 1, // entire doc
    }
}

/// Streaming tokenization result for the entire doc stream
#[instrument(skip_all)]
pub async fn tokenize_whole_doc_stream(
    request: impl Stream<Item = BidiStreamingChunkerTokenizationTaskRequest>,
) -> Result<ChunkerTokenizationStreamResult, Error> {
    let (text, index_vec): (String, Vec<i64>) = request
        .map(|r| (r.text_stream, r.input_index_stream))
        .collect()
        .await;
    let codepoint_count = text.chars().count() as i64;
    let input_end_index = index_vec.last().copied().unwrap_or_default();
    Ok(ChunkerTokenizationStreamResult {
        results: vec![Token {
            start: 0,
            end: codepoint_count,
            text,
        }],
        token_count: 1, // entire doc/stream
        processed_index: codepoint_count,
        start_index: 0,
        input_start_index: 0,
        input_end_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_whole_doc() {
        let request = ChunkerTokenizationTaskRequest {
            text: "Lorem ipsum dolor sit amet consectetur adipiscing \
            elit sed do eiusmod tempor incididunt ut labore et dolore \
            magna aliqua."
                .into(),
        };
        let expected_response = TokenizationResults {
            results: vec![Token {
                start: 0,
                end: 121,
                text: "Lorem ipsum dolor sit amet consectetur \
                    adipiscing elit sed do eiusmod tempor incididunt \
                    ut labore et dolore magna aliqua."
                    .into(),
            }],
            token_count: 1,
        };
        let response = tokenize_whole_doc(request);
        assert_eq!(response, expected_response)
    }

    #[tokio::test]
    async fn test_tokenize_whole_doc_stream() {
        let request = futures::stream::iter(vec![
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "Lorem ipsum dolor sit amet ".into(),
                input_index_stream: 0,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "consectetur adipiscing elit ".into(),
                input_index_stream: 1,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "sed do eiusmod tempor incididunt ".into(),
                input_index_stream: 2,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "ut labore et dolore magna aliqua.".into(),
                input_index_stream: 3,
            },
        ]);
        let expected_response = ChunkerTokenizationStreamResult {
            results: vec![Token {
                start: 0,
                end: 121,
                text: "Lorem ipsum dolor sit amet consectetur adipiscing elit \
                    sed do eiusmod tempor incididunt ut labore et dolore magna aliqua."
                    .into(),
            }],
            token_count: 1,
            processed_index: 121,
            start_index: 0,
            input_start_index: 0,
            input_end_index: 3,
        };
        let response = tokenize_whole_doc_stream(request).await.unwrap();
        assert_eq!(response, expected_response);
    }
}
