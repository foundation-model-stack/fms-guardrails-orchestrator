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

use common::{
    chunker::{
        CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE, CHUNKER_STREAMING_ENDPOINT,
        CHUNKER_UNARY_ENDPOINT,
    },
    detectors::{
        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC,
        DETECTOR_NAME_PARENTHESIS_SENTENCE, FACT_CHECKING_DETECTOR_SENTENCE, NON_EXISTING_DETECTOR,
        TEXT_CONTENTS_DETECTOR_ENDPOINT,
    },
    errors::{DetectorError, OrchestratorError},
    generation::{
        GENERATION_NLP_MODEL_ID_HEADER_NAME, GENERATION_NLP_STREAMING_ENDPOINT,
        GENERATION_NLP_TOKENIZATION_ENDPOINT,
    },
    orchestrator::{
        ORCHESTRATOR_CONFIG_FILE_PATH, ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE,
        ORCHESTRATOR_STREAMING_ENDPOINT, ORCHESTRATOR_UNSUITABLE_INPUT_MESSAGE, SseStream,
        TestOrchestratorServer,
    },
};
use eventsource_stream::Eventsource;
use fms_guardrails_orchestr8::{
    clients::detector::{ContentAnalysisRequest, ContentAnalysisResponse},
    models::{
        ClassifiedGeneratedTextStreamResult, DetectionWarning, DetectorParams, GuardrailsConfig,
        GuardrailsConfigInput, GuardrailsConfigOutput, GuardrailsHttpRequest, Metadata,
        TextGenTokenClassificationResults, TokenClassificationResult,
    },
    pb::{
        caikit::runtime::{
            chunkers::{
                BidiStreamingChunkerTokenizationTaskRequest, ChunkerTokenizationTaskRequest,
            },
            nlp::{ServerStreamingTextGenerationTaskRequest, TokenizationTaskRequest},
        },
        caikit_data_model::nlp::{
            ChunkerTokenizationStreamResult, GeneratedTextStreamResult, Token, TokenizationResults,
        },
    },
};
use futures::{StreamExt, TryStreamExt};
use mocktail::prelude::*;
use test_log::test;
use tracing::debug;

pub mod common;

// To troubleshoot tests with response deserialization errors, the following code
// snippet is recommended:
// // Example showing how to create an event stream from a bytes stream.
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

/// Asserts that given a request with no detectors configured returns the text generated
/// by the model.
#[test(tokio::test)]
async fn no_detectors() -> Result<(), anyhow::Error> {
    // Add generation mock
    let model_id = "my-super-model-8B";

    let mut mocks = MockSet::new();
    mocks.mock(|when, then| {
        when.path(GENERATION_NLP_STREAMING_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            });
        then.pb_stream([
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
        ]);
    });

    // Configure mock servers
    let generation_server = MockServer::new("nlp").grpc().with_mocks(mocks);

    // Run test orchestrator server
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&generation_server)
        .build()
        .await?;

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: None,
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Collects stream results
    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // assertions
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].generated_text, Some("I".into()));
    assert_eq!(messages[1].generated_text, Some(" am".into()));
    assert_eq!(messages[2].generated_text, Some(" great!".into()));

    Ok(())
}

/// Asserts that the generated text is returned when an input detector configured
/// with a sentence chunker finds no detections.
#[test(tokio::test)]
async fn input_detector_no_detections() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;

    // Add input chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb(ChunkerTokenizationTaskRequest {
                text: "Hi there! How are you?".into(),
            });
        then.pb(TokenizationResults {
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
        });
    });

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["Hi there!".into(), " How are you?".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([
            Vec::<ContentAnalysisResponse>::new(),
            Vec::<ContentAnalysisResponse>::new(),
        ]);
    });

    // Add generation mock
    let model_id = "my-super-model-8B";

    let mut generation_mocks = MockSet::new();
    generation_mocks.mock(|when, then| {
        when.header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            });
        then.pb_stream([
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
        ]);
    });

    // Start orchestrator server and its dependencies
    let mock_chunker_server = MockServer::new(chunker_id).grpc().with_mocks(chunker_mocks);
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detection_mocks);
    let generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&generation_server)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .build()
        .await?;

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
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
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // assertions
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].generated_text, Some("I".into()));
    assert!(messages[0].token_classification_results.input.is_none());

    assert_eq!(messages[1].generated_text, Some(" am".into()));
    assert!(messages[1].token_classification_results.input.is_none());

    assert_eq!(messages[2].generated_text, Some(" great!".into()));
    assert!(messages[2].token_classification_results.input.is_none());

    Ok(())
}

/// Asserts that detections found by an input detector configured with a sentence chunker
/// are returned.
#[test(tokio::test)]
async fn input_detector_detections() -> Result<(), anyhow::Error> {
    // Add chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.mock(|when, then| {
        when.header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .post()
            .path(CHUNKER_UNARY_ENDPOINT)
            .pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".into(),
            });
        then.pb(TokenizationResults {
            results: vec![
                Token {
                    start: 0,
                    end: 40,
                    text: "This sentence does not have a detection.".into(),
                },
                Token {
                    start: 41,
                    end: 61,
                    text: "But <this one does>.".into(),
                },
            ],
            token_count: 0,
        });
    });

    // Add input detection mock
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let mock_detection_response = ContentAnalysisResponse {
        start: 5,
        end: 18,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(detector_name.into()),
        score: 1.0,
        evidence: None,
        metadata: Metadata::new(),
    };
    let mut detection_mocks = MockSet::new();
    detection_mocks.mock(|when, then| {
        when.path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection.".into(),
                    "But <this one does>.".into(),
                ],
                detector_params: DetectorParams::new(),
            });
        then.json(vec![vec![], vec![mock_detection_response.clone()]]);
    });

    // Add generation mock for input token count
    let model_id = "my-super-model-8B";
    let mock_tokenization_response = TokenizationResults {
        results: Vec::new(),
        token_count: 61,
    };
    let mut generation_mocks = MockSet::new();
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_TOKENIZATION_ENDPOINT)
            .pb(TokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".into(),
            });
        then.pb(mock_tokenization_response.clone());
    });

    // Start orchestrator server and its dependencies
    let mock_chunker_server = MockServer::new(chunker_id).grpc().with_mocks(chunker_mocks);
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detection_mocks);
    let generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&generation_server)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .build()
        .await?;

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This sentence does not have a detection. But <this one does>.".into(),
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
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // assertions
    assert_eq!(messages.len(), 1);
    assert!(messages[0].generated_text.is_none());
    assert_eq!(
        messages[0].token_classification_results,
        TextGenTokenClassificationResults {
            input: Some(vec![TokenClassificationResult {
                start: 46, // index of first token of detected text, relative to the `inputs` string sent in the orchestrator request.
                end: 59, // index of last token (+1) of detected text, relative to the `inputs` string sent in the orchestrator request.
                word: mock_detection_response.text,
                entity: mock_detection_response.detection,
                entity_group: mock_detection_response.detection_type,
                detector_id: mock_detection_response.detector_id,
                score: mock_detection_response.score,
                token_count: None
            }]),
            output: None
        }
    );
    assert_eq!(
        messages[0].input_token_count,
        mock_tokenization_response.token_count as u32
    );
    assert_eq!(
        messages[0].warnings,
        Some(vec![DetectionWarning {
            id: Some(fms_guardrails_orchestr8::models::DetectionWarningReason::UnsuitableInput),
            message: Some(ORCHESTRATOR_UNSUITABLE_INPUT_MESSAGE.into())
        }])
    );

    Ok(())
}

/// Asserts that errors returned from input chunkers, input detectors and generation server are correctly propagated.
#[test(tokio::test)]
async fn input_detector_client_error() -> Result<(), anyhow::Error> {
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let model_id = "my-super-model-8B";

    let chunker_error_input = "Chunker should return an error";
    let detector_error_input = "Detector should return an error";
    let generation_server_error_input = "Generation should return an error";

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb(ChunkerTokenizationTaskRequest {
                text: chunker_error_input.into(),
            });
        then.internal_server_error();
    });
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb(ChunkerTokenizationTaskRequest {
                text: detector_error_input.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: detector_error_input.len() as i64,
                text: detector_error_input.into(),
            }],
            token_count: 0,
        });
    });
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb(ChunkerTokenizationTaskRequest {
                text: generation_server_error_input.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: generation_server_error_input.len() as i64,
                text: generation_server_error_input.into(),
            }],
            token_count: 0,
        });
    });

    let expected_detector_error = DetectorError {
        code: 500,
        message: "Internal detector error.".into(),
    };
    let mut detector_mocks = MockSet::new();
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![detector_error_input.into()],
                detector_params: DetectorParams::new(),
            });
        then.json(&expected_detector_error).internal_server_error();
    });
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![generation_server_error_input.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    let mut generation_mocks = MockSet::new();
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_STREAMING_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(ServerStreamingTextGenerationTaskRequest {
                text: generation_server_error_input.into(),
                ..Default::default()
            });
        then.internal_server_error();
    });

    // Start orchestrator server and its dependencies
    let mock_chunker_server = MockServer::new(chunker_id).grpc().with_mocks(chunker_mocks);
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detector_mocks);
    let mock_generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([&mock_detector_server])
        .generation_server(&mock_generation_server)
        .build()
        .await?;

    // Test error from chunker
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: chunker_error_input.into(),
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

    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        OrchestratorError {
            code: 500,
            details: ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE.into()
        }
    );

    // Test error from detector
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: detector_error_input.into(),
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

    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        OrchestratorError {
            code: 500,
            details: ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE.into()
        }
    );

    // Test error from generation server
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: generation_server_error_input.into(),
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

    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        OrchestratorError {
            code: 500,
            details: ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE.into()
        }
    );

    Ok(())
}

/// Asserts orchestrator request validation
#[test(tokio::test)]
async fn orchestrator_validation_error() -> Result<(), anyhow::Error> {
    let model_id = "my-super-model-8B";

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .build()
        .await?;

    // Request with extra fields scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&serde_json::json!({
            "model_id": model_id,
            "inputs": "This request does not comply with the orchestrator API",
            "guardrail_config": {
                "inputs": {},
                "outputs": {}
            },
            "non_existing_field": "random value"
        }))
        .send()
        .await?;

    debug!(?response);

    assert_eq!(response.status(), 422);
    let response_body = response.json::<OrchestratorError>().await?;
    assert_eq!(response_body.code, 422);
    assert!(
        response_body
            .details
            .starts_with("non_existing_field: unknown field `non_existing_field`")
    );

    // Invalid input detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This request contains a detector with invalid type".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(
                        FACT_CHECKING_DETECTOR_SENTENCE.into(),
                        DetectorParams::new(),
                    )]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!(?response);

    assert_eq!(response.status(), 200);
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        OrchestratorError {
            code: 422,
            details: format!(
                "detector `{}` is not supported by this endpoint",
                FACT_CHECKING_DETECTOR_SENTENCE
            )
        },
        "failed at invalid input detector scenario"
    );

    // Invalid chunker on input detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This request contains a detector with an invalid chunker".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(
                        DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(),
                        DetectorParams::new(),
                    )]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), 200);
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        OrchestratorError {
            code: 422,
            details: format!(
                "detector `{}` uses chunker `whole_doc_chunker`, which is not supported by this endpoint",
                DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC
            )
        },
        "failed on input detector with invalid chunker scenario"
    );

    // Non-existing input detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This request contains a detector with invalid type".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(NON_EXISTING_DETECTOR.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!(?response);

    assert_eq!(response.status(), 200);
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        OrchestratorError {
            code: 404,
            details: format!("detector `{}` not found", NON_EXISTING_DETECTOR)
        },
        "failed at non-existing input detector scenario"
    );

    // Invalid output detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This request contains a detector with invalid type".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(
                        FACT_CHECKING_DETECTOR_SENTENCE.into(),
                        DetectorParams::new(),
                    )]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!(?response);

    assert_eq!(response.status(), 200);
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        OrchestratorError {
            code: 422,
            details: format!(
                "detector `{}` is not supported by this endpoint",
                FACT_CHECKING_DETECTOR_SENTENCE
            )
        },
        "failed at invalid output detector scenario"
    );

    // Invalid chunker on output detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This request contains a detector with an invalid chunker".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(
                        DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(),
                        DetectorParams::new(),
                    )]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), 200);
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        OrchestratorError {
            code: 422,
            details: format!(
                "detector `{}` uses chunker `whole_doc_chunker`, which is not supported by this endpoint",
                DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC
            )
        },
        "failed on output detector with invalid chunker scenario"
    );

    // Non-existing output detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This request contains a detector with invalid type".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(NON_EXISTING_DETECTOR.into(), DetectorParams::new())]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!(?response);

    assert_eq!(response.status(), 200);
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        OrchestratorError {
            code: 404,
            details: format!("detector `{}` not found", NON_EXISTING_DETECTOR)
        },
        "failed at non-existing output detector scenario"
    );

    Ok(())
}

/// Asserts that the generated text is returned when an output detector configured
/// with a sentence chunker finds no detections.
#[test(tokio::test)]
async fn output_detectors_no_detections() -> Result<(), anyhow::Error> {
    let angle_brackets_detector = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let parenthesis_detector = DETECTOR_NAME_PARENTHESIS_SENTENCE;

    // Add generation mock
    let model_id = "my-super-model-8B";

    let mut generation_mocks = MockSet::new();
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_STREAMING_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            });
        then.pb_stream(vec![
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
            GeneratedTextStreamResult {
                generated_text: " What".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " about".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " you?".into(),
                ..Default::default()
            },
        ]);
    });

    // Add output chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "I".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " am".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " great!".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " What".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " about".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " you?".into(),
                    input_index_stream: 5,
                },
            ]);

        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 11,
                    text: "I am great!".into(),
                }],
                token_count: 0,
                processed_index: 11,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 0,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 11,
                    end: 27,
                    text: " What about you?".into(),
                }],
                token_count: 0,
                processed_index: 27,
                start_index: 11,
                input_start_index: 0,
                input_end_index: 0,
            },
        ]);
    });

    // Add output detection mock
    // TODO: Simply clone mocks instead of create two exact MockSets when/if
    // this gets merged: https://github.com/IBM/mocktail/pull/41
    let mut angle_brackets_mocks = MockSet::new();
    angle_brackets_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["I am great!".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });
    angle_brackets_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![" What about you?".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    let mut parenthesis_mocks = MockSet::new();
    parenthesis_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["I am great!".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });
    parenthesis_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![" What about you?".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Start orchestrator server and its dependencies
    let mock_chunker_server = MockServer::new(chunker_id).grpc().with_mocks(chunker_mocks);
    let mock_angle_brackets_detector_server =
        MockServer::new(angle_brackets_detector).with_mocks(angle_brackets_mocks);
    let mock_parenthesis_detector_server =
        MockServer::new(parenthesis_detector).with_mocks(parenthesis_mocks);
    let generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&generation_server)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([
            &mock_angle_brackets_detector_server,
            &mock_parenthesis_detector_server,
        ])
        .build()
        .await?;

    // Single-detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(
                        angle_brackets_detector.into(),
                        DetectorParams::new(),
                    )]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 2);

    assert_eq!(messages[0].generated_text, Some("I am great!".into()));
    assert_eq!(
        messages[0].token_classification_results.output,
        Some(vec![])
    );
    assert_eq!(messages[0].start_index, Some(0));
    assert_eq!(messages[0].processed_index, Some(11));

    assert_eq!(messages[1].generated_text, Some(" What about you?".into()));
    assert_eq!(
        messages[1].token_classification_results.output,
        Some(vec![])
    );
    assert_eq!(messages[1].start_index, Some(11));
    assert_eq!(messages[1].processed_index, Some(27));

    // Multi-detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([
                        (angle_brackets_detector.into(), DetectorParams::new()),
                        (parenthesis_detector.into(), DetectorParams::new()),
                    ]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 2);

    assert_eq!(messages[0].generated_text, Some("I am great!".into()));
    assert_eq!(
        messages[0].token_classification_results.output,
        Some(vec![])
    );
    assert_eq!(messages[0].start_index, Some(0));
    assert_eq!(messages[0].processed_index, Some(11));

    assert_eq!(messages[1].generated_text, Some(" What about you?".into()));
    assert_eq!(
        messages[1].token_classification_results.output,
        Some(vec![])
    );
    assert_eq!(messages[1].start_index, Some(11));
    assert_eq!(messages[1].processed_index, Some(27));

    Ok(())
}

/// Asserts that the generated text is returned alongside detections when an output detector
/// configured with a sentence chunker finds detections.
#[test(tokio::test)]
async fn output_detectors_detections() -> Result<(), anyhow::Error> {
    let angle_brackets_detector = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let parenthesis_detector = DETECTOR_NAME_PARENTHESIS_SENTENCE;

    // Add generation mock
    let model_id = "my-super-model-8B";
    let mut generation_mocks = MockSet::new();
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_STREAMING_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            });

        then.pb_stream(vec![
            GeneratedTextStreamResult {
                generated_text: "I".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " (am)".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " great!".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " What".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " about".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " <you>?".into(),
                ..Default::default()
            },
        ]);
    });

    // Add output chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "I".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " (am)".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " great!".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " What".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " about".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " <you>?".into(),
                    input_index_stream: 5,
                },
            ]);

        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 13,
                    text: "I (am) great!".into(),
                }],
                token_count: 0,
                processed_index: 13,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 0,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 13,
                    end: 31,
                    text: " What about <you>?".into(),
                }],
                token_count: 0,
                processed_index: 31,
                start_index: 13,
                input_start_index: 0,
                input_end_index: 0,
            },
        ]);
    });

    // Add output detection mock
    let mut angle_brackets_mocks = MockSet::new();
    angle_brackets_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["I (am) great!".into()],
                detector_params: DetectorParams::new(),
            });

        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });
    angle_brackets_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![" What about <you>?".into()],
                detector_params: DetectorParams::new(),
            });

        then.json([vec![ContentAnalysisResponse {
            start: 13,
            end: 16,
            text: "you".into(),
            detection: "has_angle_brackets".into(),
            detection_type: "angle_brackets".into(),
            detector_id: Some(angle_brackets_detector.into()),
            score: 1.0,
            evidence: None,
            metadata: Metadata::new(),
        }]]);
    });

    let mut parenthesis_mocks = MockSet::new();
    parenthesis_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["I (am) great!".into()],
                detector_params: DetectorParams::new(),
            });

        then.json([vec![ContentAnalysisResponse {
            start: 3,
            end: 5,
            text: "am".into(),
            detection: "has_parenthesis".into(),
            detection_type: "parenthesis".into(),
            detector_id: Some(parenthesis_detector.into()),
            score: 1.0,
            evidence: None,
            metadata: Metadata::new(),
        }]]);
    });
    parenthesis_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![" What about <you>?".into()],
                detector_params: DetectorParams::new(),
            });

        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Start orchestrator server and its dependencies
    let mock_chunker_server = MockServer::new(chunker_id).grpc().with_mocks(chunker_mocks);
    let mock_angle_brackets_detector_server =
        MockServer::new(angle_brackets_detector).with_mocks(angle_brackets_mocks);
    let mock_parenthesis_detector_server =
        MockServer::new(parenthesis_detector).with_mocks(parenthesis_mocks);
    let generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&generation_server)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([
            &mock_angle_brackets_detector_server,
            &mock_parenthesis_detector_server,
        ])
        .build()
        .await?;

    // Single-detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(
                        angle_brackets_detector.into(),
                        DetectorParams::new(),
                    )]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    let expected_messages = vec![
        ClassifiedGeneratedTextStreamResult {
            generated_text: Some("I (am) great!".into()),
            token_classification_results: TextGenTokenClassificationResults {
                input: None,
                output: Some(vec![]),
            },
            processed_index: Some(13),
            start_index: Some(0),
            tokens: Some(vec![]),
            input_tokens: Some(vec![]),
            ..Default::default()
        },
        ClassifiedGeneratedTextStreamResult {
            generated_text: Some(" What about <you>?".into()),
            token_classification_results: TextGenTokenClassificationResults {
                input: None,
                output: Some(vec![TokenClassificationResult {
                    start: 13,
                    end: 16,
                    word: "you".into(),
                    entity: "has_angle_brackets".into(),
                    entity_group: "angle_brackets".into(),
                    detector_id: Some(angle_brackets_detector.into()),
                    score: 1.0,
                    token_count: None,
                }]),
            },
            processed_index: Some(31),
            start_index: Some(13),
            tokens: Some(vec![]),
            input_tokens: Some(vec![]),
            ..Default::default()
        },
    ];

    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages, expected_messages,
        "failed on single-detector scenario"
    );

    // Multi-detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([
                        (angle_brackets_detector.into(), DetectorParams::new()),
                        (parenthesis_detector.into(), DetectorParams::new()),
                    ]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    let expected_messages = vec![
        ClassifiedGeneratedTextStreamResult {
            generated_text: Some("I (am) great!".into()),
            token_classification_results: TextGenTokenClassificationResults {
                input: None,
                output: Some(vec![TokenClassificationResult {
                    start: 3,
                    end: 5,
                    word: "am".into(),
                    entity: "has_parenthesis".into(),
                    entity_group: "parenthesis".into(),
                    detector_id: Some(parenthesis_detector.into()),
                    score: 1.0,
                    token_count: None,
                }]),
            },
            processed_index: Some(13),
            start_index: Some(0),
            tokens: Some(vec![]),
            input_tokens: Some(vec![]),
            ..Default::default()
        },
        ClassifiedGeneratedTextStreamResult {
            generated_text: Some(" What about <you>?".into()),
            token_classification_results: TextGenTokenClassificationResults {
                input: None,
                output: Some(vec![TokenClassificationResult {
                    start: 13,
                    end: 16,
                    word: "you".into(),
                    entity: "has_angle_brackets".into(),
                    entity_group: "angle_brackets".into(),
                    detector_id: Some(angle_brackets_detector.into()),
                    score: 1.0,
                    token_count: None,
                }]),
            },
            processed_index: Some(31),
            start_index: Some(13),
            tokens: Some(vec![]),
            input_tokens: Some(vec![]),
            ..Default::default()
        },
    ];

    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages, expected_messages,
        "failed on multi-detector scenario"
    );

    Ok(())
}

/// Asserts errors returned from output clients
#[test(tokio::test)]
async fn output_detector_client_error() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;

    // Add generation mock
    let model_id = "my-super-model-8B";
    let mut generation_mocks = MockSet::new();
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_STREAMING_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(ServerStreamingTextGenerationTaskRequest {
                text: "Make chunker return an error".into(),
                ..Default::default()
            });
        then.pb_stream(vec![
            GeneratedTextStreamResult {
                generated_text: "Chunker".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " should".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " return".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " error.".into(),
                ..Default::default()
            },
        ]);
    });
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_STREAMING_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            });
        then.pb_stream(vec![
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
            GeneratedTextStreamResult {
                generated_text: " Detector,".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " return".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " a".into(),
                ..Default::default()
            },
            GeneratedTextStreamResult {
                generated_text: " 500!".into(),
                ..Default::default()
            },
        ]);
    });

    // Add output chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Chunker".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " should".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " return".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " error.".into(),
                    input_index_stream: 3,
                },
            ]);

        then.internal_server_error();
    });
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "I".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " am".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " great!".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " Detector,".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " return".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " a".into(),
                    input_index_stream: 5,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " 500!".into(),
                    input_index_stream: 6,
                },
            ]);

        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 11,
                    text: "I am great!".into(),
                }],
                token_count: 0,
                processed_index: 11,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 0,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 11,
                    end: 35,
                    text: " Detector, return a 500!".into(),
                }],
                token_count: 0,
                processed_index: 35,
                start_index: 11,
                input_start_index: 0,
                input_end_index: 0,
            },
        ]);
    });

    // Add output detection mock
    let expected_detector_error = DetectorError {
        code: 500,
        message: "The detector service had an unexpected error.".into(),
    };
    let mut detection_mocks = MockSet::new();
    detection_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["I am great!".into()],
                detector_params: DetectorParams::new(),
            });

        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });
    detection_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![" Detector, return a 500!".into()],
                detector_params: DetectorParams::new(),
            });

        then.json(&expected_detector_error).internal_server_error();
    });

    // Start orchestrator server and its dependencies
    let mock_chunker_server = MockServer::new(chunker_id).grpc().with_mocks(chunker_mocks);
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detection_mocks);
    let generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&generation_server)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // assert chunker error
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Make chunker return an error".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        OrchestratorError {
            code: 500,
            details: ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE.into()
        }
    );

    // assert detector error
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    debug!("{response:#?}");

    let mut events = Vec::new();
    let mut event_stream = response.bytes_stream().eventsource();
    while let Some(event) = event_stream.next().await {
        match event {
            Ok(event) => {
                if event.data == "[DONE]" {
                    break;
                }
                debug!("recv: {event:?}");
                events.push(event.data);
            }
            Err(_) => {
                panic!("received error from event stream");
            }
        }
    }
    debug!("{events:?}");

    let first_response =
        serde_json::from_str::<ClassifiedGeneratedTextStreamResult>(events[0].as_str())?;
    let second_response = serde_json::from_str::<OrchestratorError>(events[1].as_str())?;

    assert_eq!(events.len(), 2);
    assert_eq!(first_response.generated_text, Some("I am great!".into()));
    assert_eq!(
        first_response.token_classification_results.output,
        Some(vec![])
    );
    assert_eq!(first_response.start_index, Some(0));
    assert_eq!(first_response.processed_index, Some(11));

    assert_eq!(
        second_response,
        OrchestratorError {
            code: 500,
            details: ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE.into()
        }
    );

    Ok(())
}
