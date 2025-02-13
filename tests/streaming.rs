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

use common::{
    generation::{MockNlpServiceServer, GENERATION_NLP_STREAMING_ENDPOINT},
    util::{ensure_global_rustls_state, SseStream, TestOrchestratorServer},
};
use fms_guardrails_orchestr8::{
    clients::nlp::MODEL_ID_HEADER_NAME,
    models::{ClassifiedGeneratedTextStreamResult, GuardrailsHttpRequest},
    pb::{
        caikit::runtime::nlp::ServerStreamingTextGenerationTaskRequest,
        caikit_data_model::nlp::GeneratedTextStreamResult,
    },
};
use futures::StreamExt;
use mocktail::prelude::*;
use tracing_test::traced_test;

pub mod common;

const STREAMING_CLASSIFICATION_WITH_GEN_ENDPOINT: &str =
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

    // Configure mock servers
    let generation_server = MockNlpServiceServer::new(mocks)?;

    // Run test orchestrator server
    let orchestrator_server = TestOrchestratorServer::run(
        "tests/test.config.yaml",
        8080,
        8081,
        Some(generation_server),
        None,
        None,
        None,
    )
    .await?;

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(STREAMING_CLASSIFICATION_WITH_GEN_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.to_string(),
            inputs: "Hi there! How are you?".to_string(),
            guardrail_config: None,
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Example showing how to create an event stream from a bytes stream.
    // let mut events = Vec::new();
    // let mut event_stream = response.bytes_stream().eventsource();
    // while let Some(event) = event_stream.next().await {
    //     match event {
    //         Ok(event) => {
    //             if event.data == "[DONE]" {
    //                 break;
    //             }
    //             println!("recv: {event:?}");
    //             events.push(event.data);
    //         }
    //         Err(_) => {
    //             panic!("received error from event stream");
    //         }
    //     }
    // }
    // println!("{events:?}");

    // Test custom SseStream wrapper
    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    println!("{messages:?}");

    // assertions
    assert!(messages.len() == 3);
    assert!(messages[0].generated_text == Some("I".into()));
    assert!(messages[1].generated_text == Some(" am".into()));
    assert!(messages[2].generated_text == Some(" great!".into()));

    Ok(())
}
