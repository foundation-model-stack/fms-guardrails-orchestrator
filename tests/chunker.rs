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

use common::chunker::{MockChunkersServiceServer, CHUNKER_UNARY_ENDPOINT};
use fms_guardrails_orchestr8::{
    clients::chunker::{ChunkerClient, MODEL_ID_HEADER_NAME},
    config::ServiceConfig,
    pb::{
        caikit::runtime::chunkers::ChunkerTokenizationTaskRequest,
        caikit_data_model::nlp::{Token, TokenizationResults},
    },
};
use mocktail::prelude::*;
use tracing_test::traced_test;

pub mod common;

#[traced_test]
#[tokio::test]
async fn test_isolated_chunker_unary_call() -> Result<(), anyhow::Error> {
    // Add detector mock
    let chunker_id = "sentence_chunker";
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(MODEL_ID_HEADER_NAME, chunker_id.parse().unwrap());

    let expected_response = TokenizationResults {
        results: vec![
            Token {
                start: 0,
                end: 9,
                text: "Hi there!".to_string(),
            },
            Token {
                start: 0,
                end: 9,
                text: "how are you?".to_string(),
            },
            Token {
                start: 0,
                end: 9,
                text: "I am great!".to_string(),
            },
        ],
        token_count: 0,
    };

    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "Hi there! how are you? I am great!".to_string(),
            })
            .with_headers(chunker_headers),
            MockResponse::pb(expected_response.clone()),
        ),
    );

    let mock_chunker_server = MockChunkersServiceServer::new(mocks)?;
    let _ = mock_chunker_server.start().await;

    let client = ChunkerClient::new(&ServiceConfig {
        hostname: "localhost".to_string(),
        port: Some(mock_chunker_server.addr().port()),
        request_timeout: None,
        tls: None,
    })
    .await;

    let response = client
        .tokenization_task_predict(
            chunker_id,
            ChunkerTokenizationTaskRequest {
                text: "Hi there! how are you? I am great!".to_string(),
            },
        )
        .await;

    dbg!(&response);

    assert!(response.is_ok());
    assert!(response? == expected_response);

    Ok(())
}
