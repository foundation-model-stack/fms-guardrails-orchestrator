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

use axum_test::TestServer;
use common::{
    generation::{MockNlpServiceServer, GENERATION_NLP_STREAMING_ENDPOINT},
    util::{create_orchestrator_shared_state, ensure_global_rustls_state},
};
use fms_guardrails_orchestr8::{
    clients::nlp::MODEL_ID_HEADER_NAME,
    models::{ClassifiedGeneratedTextStreamResult, GuardrailsHttpRequest},
    pb::{
        caikit::runtime::nlp::ServerStreamingTextGenerationTaskRequest,
        caikit_data_model::nlp::GeneratedTextStreamResult,
    },
    server::get_app,
};
use mocktail::prelude::*;
use tracing::debug;
use tracing_test::traced_test;

pub mod common;

const ENDPOINT_ORCHESTRATOR: &str =
    "/api/v1/task/server-streaming-classification-with-text-generation";

#[traced_test]
#[tokio::test]
async fn test_no_detectors() -> Result<(), anyhow::Error> {
    ensure_global_rustls_state();

    // Add generation mock
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

    // Setup servers
    let generation_nlp_server = MockNlpServiceServer::new(mocks)?;
    let shared_state =
        create_orchestrator_shared_state(Some(&generation_nlp_server), vec![], vec![]).await?;
    let server = TestServer::new(get_app(shared_state))?;

    // Make orchestrator call
    let response = server
        .post(ENDPOINT_ORCHESTRATOR)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.to_string(),
            inputs: "Hi there! How are you?".to_string(),
            guardrail_config: None,
            text_gen_parameters: None,
        })
        .await;

    // convert SSE events back into Rust structs
    let text = response.text();
    let results: Vec<_> = text
        .split("\n\n")
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str::<ClassifiedGeneratedTextStreamResult>(&line.replace("data: ", ""))
                .unwrap()
        })
        .collect();

    // assertions
    assert!(results.len() == 3);
    assert!(results[0].generated_text == Some("I".into()));
    assert!(results[1].generated_text == Some(" am".into()));
    assert!(results[2].generated_text == Some(" great!".into()));

    debug!("{:#?}", results);

    Ok(())
}
