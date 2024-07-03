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

use std::{collections::HashMap, pin::Pin};

use futures::{Stream, StreamExt};
use ginepro::LoadBalancedChannel;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Request;
use tracing::info;

use super::{create_grpc_clients, Error};
use crate::{
    config::ServiceConfig,
    pb::{
        caikit::runtime::chunkers::{
            chunkers_service_client::ChunkersServiceClient, BidiStreamingTokenizationTaskRequest,
            TokenizationTaskRequest,
        },
        caikit_data_model::nlp::{Token, TokenizationResults, TokenizationStreamResult},
    },
};

const MODEL_ID_HEADER_NAME: &str = "mm-model-id";
/// Default chunker that returns span for entire text
const DEFAULT_MODEL_ID: &str = "whole_doc_chunker";

#[cfg_attr(test, derive(Default))]
#[derive(Clone)]
pub struct ChunkerClient {
    clients: HashMap<String, ChunkersServiceClient<LoadBalancedChannel>>,
}

impl ChunkerClient {
    pub async fn new(default_port: u16, config: &[(String, ServiceConfig)]) -> Self {
        let clients = create_grpc_clients(default_port, config, ChunkersServiceClient::new).await;
        Self { clients }
    }

    fn client(&self, model_id: &str) -> Result<ChunkersServiceClient<LoadBalancedChannel>, Error> {
        Ok(self
            .clients
            .get(model_id)
            .ok_or_else(|| Error::ModelNotFound {
                model_id: model_id.to_string(),
            })?
            .clone())
    }

    pub async fn tokenization_task_predict(
        &self,
        model_id: &str,
        request: TokenizationTaskRequest,
    ) -> Result<TokenizationResults, Error> {
        // Handle "default" separately first
        if model_id == DEFAULT_MODEL_ID {
            info!("Using default whole doc chunker");
            return Ok(tokenize_whole_doc(request));
        }
        let request = request_with_model_id(request, model_id);
        Ok(self
            .client(model_id)?
            .tokenization_task_predict(request)
            .await?
            .into_inner())
    }

    pub async fn bidi_streaming_tokenization_task_predict(
        &self,
        model_id: &str,
        request: Pin<Box<dyn Stream<Item = BidiStreamingTokenizationTaskRequest> + Send + 'static>>,
    ) -> Result<ReceiverStream<TokenizationStreamResult>, Error> {
        let (tx, rx) = mpsc::channel(128);
        // Handle "default" separately first
        if model_id == DEFAULT_MODEL_ID {
            info!("Using default whole doc chunker");
            let _ = tx.send(tokenize_whole_doc_stream(request).await).await;
            return Ok(ReceiverStream::new(rx));
        }
        let request = request_with_model_id(request, model_id);
        let mut response_stream = self
            .client(model_id)?
            .bidi_streaming_tokenization_task_predict(request)
            .await?
            .into_inner();
        tokio::spawn(async move {
            while let Some(Ok(message)) = response_stream.next().await {
                let _ = tx.send(message).await;
            }
        });
        Ok(ReceiverStream::new(rx))
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
fn tokenize_whole_doc(request: TokenizationTaskRequest) -> TokenizationResults {
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
async fn tokenize_whole_doc_stream(
    request: impl Stream<Item = BidiStreamingTokenizationTaskRequest>,
) -> TokenizationStreamResult {
    let text = request.map(|r| r.text_stream).collect::<String>().await;
    let codepoint_count = text.chars().count() as i64;
    TokenizationStreamResult {
        results: vec![Token {
            start: 0,
            end: codepoint_count,
            text,
        }],
        token_count: 1, // entire doc/stream
        processed_index: codepoint_count,
        start_index: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_whole_doc() {
        let request = TokenizationTaskRequest {
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
            BidiStreamingTokenizationTaskRequest {
                text_stream: "Lorem ipsum dolor sit amet ".into(),
            },
            BidiStreamingTokenizationTaskRequest {
                text_stream: "consectetur adipiscing elit ".into(),
            },
            BidiStreamingTokenizationTaskRequest {
                text_stream: "sed do eiusmod tempor incididunt ".into(),
            },
            BidiStreamingTokenizationTaskRequest {
                text_stream: "ut labore et dolore magna aliqua.".into(),
            },
        ]);
        let expected_response = TokenizationStreamResult {
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
        };
        let response = tokenize_whole_doc_stream(request).await;
        assert_eq!(response, expected_response);
    }
}
