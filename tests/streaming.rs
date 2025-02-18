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

use std::collections::HashMap;
use test_log::test;

use common::{
    chunker::{MockChunkersServiceServer, CHUNKER_NAME_SENTENCE, CHUNKER_UNARY_ENDPOINT},
    detectors::{
        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC,
        TEXT_CONTENTS_DETECTOR_ENDPOINT,
    },
    errors::{DetectorError, OrchestratorError},
    generation::{MockNlpServiceServer, GENERATION_NLP_STREAMING_ENDPOINT},
    orchestrator::{
        ensure_global_rustls_state, SseStream, TestOrchestratorServer,
        ORCHESTRATOR_CONFIG_FILE_PATH,
    },
};
use fms_guardrails_orchestr8::{
    clients::{
        chunker::MODEL_ID_HEADER_NAME as CHUNKER_MODEL_ID_HEADER_NAME,
        detector::{ContentAnalysisRequest, ContentAnalysisResponse},
        nlp::MODEL_ID_HEADER_NAME as NLP_MODEL_ID_HEADER_NAME,
    },
    models::{
        ClassifiedGeneratedTextStreamResult, DetectorParams, GuardrailsConfig,
        GuardrailsConfigInput, GuardrailsHttpRequest,
    },
    pb::{
        caikit::runtime::{
            chunkers::ChunkerTokenizationTaskRequest, nlp::ServerStreamingTextGenerationTaskRequest,
        },
        caikit_data_model::nlp::{GeneratedTextStreamResult, Token, TokenizationResults},
    },
};
use futures::StreamExt;
use mocktail::{prelude::*, utils::find_available_port};
use tracing::debug;

pub mod common;

const STREAMING_CLASSIFICATION_WITH_GEN_ENDPOINT: &str =
    "/api/v1/task/server-streaming-classification-with-text-generation";

#[test(tokio::test)]
async fn test_no_detectors() -> Result<(), anyhow::Error> {
    ensure_global_rustls_state();

    // Add generation mock
    let model_id = "my-super-model-8B";
    let mut headers = HeaderMap::new();
    headers.insert(NLP_MODEL_ID_HEADER_NAME, model_id.parse().unwrap());

    let expected_response = vec![
        GeneratedTextStreamResult {
            generated_text: "I".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " am".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " great!".into(),
            ..Default::default()
        },
    ];

    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, GENERATION_NLP_STREAMING_ENDPOINT),
        Mock::new(
            MockRequest::pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
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
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
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
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
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
    debug!(?messages);

    // assertions
    assert!(messages.len() == 3);
    assert!(messages[0].generated_text == Some("I".into()));
    assert!(messages[1].generated_text == Some(" am".into()));
    assert!(messages[2].generated_text == Some(" great!".into()));

    Ok(())
}

#[test(tokio::test)]
async fn test_input_detector_whole_doc_no_detections() -> Result<(), anyhow::Error> {
    ensure_global_rustls_state();
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["Hi there! How are you?".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([Vec::<ContentAnalysisResponse>::new()]),
        ),
    );

    // Add generation mock
    let model_id = "my-super-model-8B";
    let mut headers = HeaderMap::new();
    headers.insert(NLP_MODEL_ID_HEADER_NAME, model_id.parse().unwrap());

    let expected_response = vec![
        GeneratedTextStreamResult {
            generated_text: "I".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " am".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " great!".into(),
            ..Default::default()
        },
    ];

    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(Method::POST, GENERATION_NLP_STREAMING_ENDPOINT),
        Mock::new(
            MockRequest::pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb_stream(expected_response.clone()),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
    let generation_server = MockNlpServiceServer::new(generation_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(generation_server),
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(STREAMING_CLASSIFICATION_WITH_GEN_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Test custom SseStream wrapper
    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 3);
    assert!(messages[0].generated_text == Some("I".into()));
    assert!(messages[1].generated_text == Some(" am".into()));
    assert!(messages[2].generated_text == Some(" great!".into()));

    Ok(())
}

#[test(tokio::test)]
async fn test_input_detector_sentence_chunker_no_detections() -> Result<(), anyhow::Error> {
    ensure_global_rustls_state();
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;

    // Add input chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::new(Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "Hi there! How are you?".into(),
            })
            .with_headers(chunker_headers),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 9,
                        text: "Hi there!".into(),
                    },
                    Token {
                        start: 10,
                        end: 22,
                        text: " How are you?".into(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["Hi there!".into(), " How are you?".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([
                Vec::<ContentAnalysisResponse>::new(),
                Vec::<ContentAnalysisResponse>::new(),
            ]),
        ),
    );

    // Add generation mock
    let model_id = "my-super-model-8B";
    let mut headers = HeaderMap::new();
    headers.insert(NLP_MODEL_ID_HEADER_NAME, model_id.parse().unwrap());

    let expected_response = vec![
        GeneratedTextStreamResult {
            generated_text: "I".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " am".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " great!".into(),
            ..Default::default()
        },
    ];

    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(Method::POST, GENERATION_NLP_STREAMING_ENDPOINT),
        Mock::new(
            MockRequest::pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb_stream(expected_response.clone()),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_chunker_server = MockChunkersServiceServer::new(chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
    let generation_server = MockNlpServiceServer::new(generation_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(generation_server),
        None,
        Some(vec![mock_detector_server]),
        Some(vec![(chunker_id.into(), mock_chunker_server)]),
    )
    .await?;

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(STREAMING_CLASSIFICATION_WITH_GEN_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // Test custom SseStream wrapper
    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 3);
    assert!(messages[0].generated_text == Some("I".into()));
    assert!(messages[1].generated_text == Some(" am".into()));
    assert!(messages[2].generated_text == Some(" great!".into()));

    Ok(())
}

#[test(tokio::test)]
async fn test_input_detector_returns_404() -> Result<(), anyhow::Error> {
    ensure_global_rustls_state();
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;
    let model_id = "my-super-model-8B";
    let expected_detector_error = DetectorError {
        code: 404,
        message: "Not found.".into(),
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 404".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::NOT_FOUND),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(STREAMING_CLASSIFICATION_WITH_GEN_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This should return a 404".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    debug!(?response, "RESPONSE RECEIVED FROM ORCHESTRATOR");

    // Test custom SseStream wrapper
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 1);
    assert!(messages[0].code == 404);
    assert!(
        messages[0].details
            == format!(
                "detector request failed for `{}`: {}",
                detector_name, expected_detector_error.message
            )
    );

    Ok(())
}
