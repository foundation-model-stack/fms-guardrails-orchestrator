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
use futures::{Future, Stream, StreamExt, TryStreamExt};
use ginepro::LoadBalancedChannel;
use tonic::{Code, Request, Response, Status, Streaming};

use super::{create_grpc_client, errors::grpc_to_http_code, BoxStream, Client, Error};
use crate::{
    config::ServiceConfig,
    health::{HealthCheckResult, HealthStatus},
    pb::{
        caikit::runtime::chunkers::{
            chunkers_service_client::ChunkersServiceClient,
            BidiStreamingChunkerTokenizationTaskRequest, ChunkerTokenizationTaskRequest,
        },
        caikit_data_model::nlp::{ChunkerTokenizationStreamResult, Token, TokenizationResults},
        grpc::health::v1::{health_client::HealthClient, HealthCheckRequest},
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
    client: ChunkersServiceClient<LoadBalancedChannel>,
    health_client: HealthClient<LoadBalancedChannel>,
}

#[cfg_attr(test, faux::methods)]
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
        let request = request_with_model_id(request, model_id);
        Ok(client
            .chunker_tokenization_task_predict(request)
            .await?
            .into_inner())
    }

    pub async fn bidi_streaming_tokenization_task_predict(
        &self,
        model_id: &str,
        request_stream: BoxStream<BidiStreamingChunkerTokenizationTaskRequest>,
    ) -> Result<BoxStream<Result<ChunkerTokenizationStreamResult, Error>>, Error> {
        let mut client = self.client.clone();
        let request = request_with_model_id(request_stream, model_id);
        // NOTE: this is an ugly workaround to avoid bogus higher-ranked lifetime errors.
        // https://github.com/rust-lang/rust/issues/110338
        let response_stream_fut: Pin<Box<dyn Future<Output = StreamingTokenizationResult> + Send>> =
            Box::pin(client.bidi_streaming_chunker_tokenization_task_predict(request));
        let response_stream = response_stream_fut
            .await?
            .into_inner()
            .map_err(Into::into)
            .boxed();

        Ok(response_stream)
    }
}

#[cfg_attr(test, faux::methods)]
#[async_trait]
impl Client for ChunkerClient {
    fn name(&self) -> &str {
        "chunker"
    }

    async fn health(&self) -> HealthCheckResult {
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

fn request_with_model_id<T>(request: T, model_id: &str) -> Request<T> {
    let mut request = Request::new(request);
    request
        .metadata_mut()
        .insert(MODEL_ID_HEADER_NAME, model_id.parse().unwrap());
    request
}

/// Unary tokenization result of the entire doc
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
