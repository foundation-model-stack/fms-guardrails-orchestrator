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

use common::generation::{MockNlpServiceServer, GENERATION_NLP_STREAMING_ENDPOINT};
use fms_guardrails_orchestr8::{
    clients::{nlp::MODEL_ID_HEADER_NAME, NlpClient},
    config::ServiceConfig,
    pb::{
        caikit::runtime::nlp::ServerStreamingTextGenerationTaskRequest,
        caikit_data_model::nlp::GeneratedTextStreamResult,
    },
};
use futures::StreamExt;
use mocktail::prelude::*;
use tracing_test::traced_test;

pub mod common;

#[traced_test]
#[tokio::test]
async fn test_nlp_streaming_call() -> Result<(), anyhow::Error> {
    // Add detector mock
    let model_id = "my-super-model-8B";
    let mut headers = HeaderMap::new();
    headers.insert(MODEL_ID_HEADER_NAME, model_id.parse().unwrap());

    let expected_response = vec![
        GeneratedTextStreamResult {
            generated_text: "I".to_string(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " am".to_string(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " great!".to_string(),
            ..Default::default()
        },
    ];

    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, GENERATION_NLP_STREAMING_ENDPOINT),
        Mock::new(
            MockRequest::pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".to_string(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb_stream(expected_response.clone()),
        ),
    );

    let generation_nlp_server = MockNlpServiceServer::new(mocks)?;
    generation_nlp_server.start().await?;

    let client = NlpClient::new(&ServiceConfig {
        hostname: "localhost".to_string(),
        port: Some(generation_nlp_server.addr().port()),
        request_timeout: None,
        tls: None,
    })
    .await;

    let response = client
        .server_streaming_text_generation_task_predict(
            model_id,
            ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".to_string(),
                ..Default::default()
            },
            headers,
        )
        .await?;

    // Collect stream results as array for assertion
    let result = response.map(Result::unwrap).collect::<Vec<_>>().await;

    assert!(result == expected_response);

    Ok(())
}
