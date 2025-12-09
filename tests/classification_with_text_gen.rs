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

use std::{collections::HashMap, vec};

use anyhow::Ok;
use common::{
    chunker::CHUNKER_UNARY_ENDPOINT,
    detectors::{
        ANSWER_RELEVANCE_DETECTOR_SENTENCE, DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE,
        DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, NON_EXISTING_DETECTOR,
        TEXT_CONTENTS_DETECTOR_ENDPOINT,
    },
    errors::DetectorError,
    generation::{
        GENERATION_NLP_MODEL_ID_HEADER_NAME, GENERATION_NLP_TOKENIZATION_ENDPOINT,
        GENERATION_NLP_UNARY_ENDPOINT,
    },
    orchestrator::{
        ORCHESTRATOR_CONFIG_FILE_PATH, ORCHESTRATOR_UNARY_ENDPOINT,
        ORCHESTRATOR_UNSUITABLE_INPUT_MESSAGE, TestOrchestratorServer,
    },
};
use fms_guardrails_orchestr8::{
    clients::{
        chunker::MODEL_ID_HEADER_NAME as CHUNKER_MODEL_ID_HEADER_NAME,
        detector::{ContentAnalysisRequest, ContentAnalysisResponse},
    },
    models::{
        ClassifiedGeneratedTextResult, DetectionWarning, DetectionWarningReason, DetectorParams,
        GuardrailsConfig, GuardrailsConfigInput, GuardrailsConfigOutput, GuardrailsHttpRequest,
        Metadata, TextGenTokenClassificationResults, TokenClassificationResult,
    },
    pb::{
        caikit::runtime::{
            chunkers::ChunkerTokenizationTaskRequest,
            nlp::{TextGenerationTaskRequest, TokenizationTaskRequest},
        },
        caikit_data_model::nlp::{GeneratedTextResult, Token, TokenizationResults},
    },
    server,
};
use hyper::StatusCode;
use mocktail::prelude::*;
use test_log::test;
use tracing::debug;

pub mod common;

// Constants
const CHUNKER_NAME_SENTENCE: &str = "sentence_chunker";
const MODEL_ID: &str = "my-super-model-8B";

#[test(tokio::test)]
async fn no_detectors() -> Result<(), anyhow::Error> {
    // Add generation mock
    let model_id = "my-super-model-8B";
    let inputs = "Hi there! How are you?";

    // Add expected generated text
    let expected_response = GeneratedTextResult {
        generated_text: "I am great!".into(),
        generated_tokens: 0,
        finish_reason: 0,
        input_token_count: 0,
        seed: 0,
        tokens: vec![],
        input_tokens: vec![],
    };

    let mut mocks = MockSet::new();
    mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID)
            .pb(TextGenerationTaskRequest {
                text: inputs.into(),
                ..Default::default()
            });
        then.pb(expected_response.clone());
    });

    // Configure mock servers
    let generation_server = MockServer::new_grpc("nlp").with_mocks(mocks);

    // Run test orchestrator server
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&generation_server)
        .build()
        .await?;

    // Empty `guardrail_config` scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: inputs.into(),
            guardrail_config: None,
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert_eq!(
        results.generated_text,
        Some(expected_response.clone().generated_text)
    );
    assert_eq!(results.warnings, None);

    // `guardrail_config` with `input` and `output` set to None scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert_eq!(
        results.generated_text,
        Some(expected_response.clone().generated_text)
    );
    assert_eq!(results.warnings, None);

    // `guardrail_config` with `input` and `output` set to empty map scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::new(),
                    masks: None,
                }),
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::new(),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert_eq!(
        results.generated_text,
        Some(expected_response.clone().generated_text)
    );
    assert_eq!(results.warnings, None);

    Ok(())
}

// Validate that requests without detectors, input detector and output detector configured
// returns text generated by model
#[test(tokio::test)]
async fn no_detections() -> Result<(), anyhow::Error> {
    // Add expected generated text
    let expected_response = GeneratedTextResult {
        generated_text: "I am great!".into(),
        generated_tokens: 0,
        finish_reason: 0,
        input_token_count: 0,
        seed: 0,
        tokens: vec![],
        input_tokens: vec![],
    };

    let text_mock_input = "Hi there! How are you?".to_string();

    // Create MockSets for all mocks
    let mut generation_mocks = MockSet::new();
    let mut detector_mocks = MockSet::new();
    let mut chunker_mocks = MockSet::new();

    // Add generation mock
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID)
            .pb(TextGenerationTaskRequest {
                text: text_mock_input.clone(),
                ..Default::default()
            });
        then.pb(expected_response.clone());
    });

    // Add input detector mock
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![text_mock_input.clone()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Add output detector mock
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("x-model-name", DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec![expected_response.generated_text.clone()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Add chunker tokenization for input mock
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: text_mock_input.clone(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: 22,
                text: text_mock_input.clone(),
            }],
            token_count: 0,
        });
    });

    // Add chunker tokenization for generated text
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: expected_response.generated_text.clone(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: 11,
                text: expected_response.generated_text.clone(),
            }],
            token_count: 0,
        });
    });

    // Configure mock servers
    let mock_detector_server =
        MockServer::new_http(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE).with_mocks(detector_mocks);
    let mock_generation_server = MockServer::new_grpc("nlp").with_mocks(generation_mocks);
    let mock_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE).with_mocks(chunker_mocks);

    // Run test orchestrator server
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([&mock_detector_server])
        .generation_server(&mock_generation_server)
        .build()
        .await?;

    // Orchestrator request with no input/output detectors for no detectors scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: text_mock_input.clone(),
            guardrail_config: None,
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions on no detectors scenario
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert_eq!(
        results.generated_text,
        Some(expected_response.clone().generated_text)
    );
    assert_eq!(results.warnings, None);

    // Orchestrator request with input detector for no input detections scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: text_mock_input.clone(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(
                        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(),
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

    // Assertions on no input detections scenario
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert_eq!(
        results.generated_text,
        Some(expected_response.clone().generated_text)
    );
    assert_eq!(results.warnings, None);

    // Orchestrator request with output detector for no output detections scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: text_mock_input.clone(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(
                        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(),
                        DetectorParams::new(),
                    )]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions on output no detections scenario
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert_eq!(
        results.generated_text,
        Some(expected_response.generated_text)
    );
    assert_eq!(results.warnings, None);

    Ok(())
}

// Validates that requests with input detector configured returns detections
#[test(tokio::test)]
async fn input_detector_detections() -> Result<(), anyhow::Error> {
    // Add tokenization results mock for input detections scenarios
    let tokenization_results = [
        Token {
            start: 0,
            end: 40,
            text: "This sentence does not have a detection.".to_string(),
        },
        Token {
            start: 41,
            end: 61,
            text: "But <this one does>.".to_string(),
        },
        Token {
            start: 62,
            end: 84,
            text: "Also <this other one>.".to_string(),
        },
    ];

    // Add tokenization mock responses for input detections
    let mock_tokenization_responses = [
        TokenizationResults {
            results: Vec::new(),
            token_count: 61,
        },
        TokenizationResults {
            results: Vec::new(),
            token_count: 84,
        },
    ];

    // Add input detection mock response for input detections
    let expected_detections = [
        ContentAnalysisResponse {
            start: 5,
            end: 18,
            text: "this one does".into(),
            detection: "has_angle_brackets".into(),
            detection_type: "angle_brackets".into(),
            detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into()),
            score: 1.0,
            evidence: None,
            metadata: Metadata::new(),
        },
        ContentAnalysisResponse {
            start: 6,
            end: 20,
            text: "this other one".into(),
            detection: "has_angle_brackets".into(),
            detection_type: "angle_brackets".into(),
            detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into()),
            score: 1.0,
            evidence: None,
            metadata: Metadata::new(),
        },
    ];

    let mut generation_mocks = MockSet::new();
    let mut detector_mocks = MockSet::new();
    let mut chunker_mocks = MockSet::new();

    // Add generation tokenization mock for single detection
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_TOKENIZATION_ENDPOINT)
            .pb(TokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".into(),
            });
        then.pb(mock_tokenization_responses[0].clone());
    });

    // Add generation tokenization mock for input multiple detections
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_TOKENIZATION_ENDPOINT)
            .pb(TokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
            });
        then.pb(mock_tokenization_responses[1].clone());
    });

    // Add chunker tokenization mock for input single detection
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".to_string(),
            });
        then.pb(TokenizationResults {
            results: vec![
                tokenization_results[0].clone(),
                tokenization_results[1].clone(),
            ],
            token_count: 0,
        });
    });

    // Add chunker tokenization mock for input multiple detections
    chunker_mocks.mock( |when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".to_string(),
            });
        then.pb(TokenizationResults {
            results: vec![tokenization_results[0].clone(), tokenization_results[1].clone(), tokenization_results[2].clone()],
            token_count: 0,
        });
    });

    // Add input detection mock for single detection
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection.".into(),
                    "But <this one does>.".into(),
                ],
                detector_params: DetectorParams::new(),
            });
        then.json([vec![], vec![&expected_detections[0]]]);
    });

    // Add input detection mock for multiple detections
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection.".into(),
                    "But <this one does>.".into(),
                    "Also <this other one>.".into(),
                ],
                detector_params: DetectorParams::new(),
            });
        then.json([
            vec![],
            vec![&expected_detections[0]],
            vec![&expected_detections[1]],
        ]);
    });

    // Configure mock servers
    let mock_generation_server = MockServer::new_grpc("nlp").with_mocks(generation_mocks);
    let mock_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE).with_mocks(chunker_mocks);
    let mock_detector_server =
        MockServer::new_http(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE).with_mocks(detector_mocks);

    // Run test orchestrator server
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([&mock_detector_server])
        .generation_server(&mock_generation_server)
        .build()
        .await?;

    // Orchestrator request with unary response for input single detection
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This sentence does not have a detection. But <this one does>.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(
                        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(),
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

    // Assertions for input single detection
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert_eq!(results.generated_text, None);
    assert_eq!(
        results.token_classification_results,
        TextGenTokenClassificationResults {
            input: Some(vec![TokenClassificationResult {
                start: 46_u32,
                end: 59_u32,
                word: expected_detections[0].text.clone(),
                entity: expected_detections[0].detection.clone(),
                entity_group: expected_detections[0].detection_type.clone(),
                detector_id: expected_detections[0].detector_id.clone(),
                score: expected_detections[0].score,
                token_count: None
            }]),
            output: None
        }
    );
    assert_eq!(
        results.input_token_count,
        mock_tokenization_responses[0].token_count as u32
    );
    assert_eq!(
        results.warnings,
        Some(vec![DetectionWarning {
            id: Some(DetectionWarningReason::UnsuitableInput),
            message: Some(ORCHESTRATOR_UNSUITABLE_INPUT_MESSAGE.into())
        }])
    );

    // Orchestrator request with unary response for input multiple detections
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions for input multiple detections
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert_eq!(results.generated_text, None);
    assert_eq!(
        results.token_classification_results,
        TextGenTokenClassificationResults {
            input: Some(vec![
                TokenClassificationResult {
                    start: 46_u32,
                    end: 59_u32,
                    word: expected_detections[0].text.clone(),
                    entity: expected_detections[0].detection.clone(),
                    entity_group: expected_detections[0].detection_type.clone(),
                    detector_id: expected_detections[0].detector_id.clone(),
                    score: expected_detections[0].score,
                    token_count: None
                },
                TokenClassificationResult {
                    start: 68_u32,
                    end: 82_u32,
                    word: expected_detections[1].text.clone(),
                    entity: expected_detections[1].detection.clone(),
                    entity_group: expected_detections[1].detection_type.clone(),
                    detector_id: expected_detections[1].detector_id.clone(),
                    score: expected_detections[1].score,
                    token_count: None
                }
            ]),
            output: None,
        }
    );

    Ok(())
}

// Validates that requests with input detector configured returns propagated errors
// from detector, chunker and generation server when applicable
#[test(tokio::test)]
async fn input_detector_client_error() -> Result<(), anyhow::Error> {
    // Add 500 expected input detection mock response
    let expected_detector_error = DetectorError {
        code: 500,
        message: "Internal detector error.".into(),
    };

    let orchestrator_error_500 = server::Error {
        code: http::StatusCode::INTERNAL_SERVER_ERROR,
        details: "unexpected error occurred while processing request".into(),
    };

    // Add input for error scenarios
    let chunker_error_input = "This should return a 500 error on chunker";
    let detector_error_input = "This should return a 500 error on detector";
    let generation_server_error_input = "This should return a 500 error on generation";

    let mut generation_mocks = MockSet::new();
    let mut detector_mocks = MockSet::new();
    let mut chunker_mocks = MockSet::new();

    // Add input detection mock for generation internal server error scenario
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![generation_server_error_input.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });
    // Add input detection mock for detector internal server error scenario
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![detector_error_input.into()],
                detector_params: DetectorParams::new(),
            });
        then.internal_server_error().json(&expected_detector_error);
    });
    // Add generation mock for generation internal server error scenario
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_TOKENIZATION_ENDPOINT)
            .pb(TokenizationTaskRequest {
                text: generation_server_error_input.into(),
            });
        then.internal_server_error();
    });

    // Add chunker tokenization mock for detector internal server error scenario
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
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

    // Add chunker tokenization mock for chunker internal server error scenario
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: chunker_error_input.into(),
            });
        then.internal_server_error();
    });

    // Configure mock servers
    let mock_generation_server = MockServer::new_grpc("nlp").with_mocks(generation_mocks);
    let mock_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE).with_mocks(chunker_mocks);
    let mock_detector_server =
        MockServer::new_http(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE).with_mocks(detector_mocks);

    // Run test orchestrator server
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([&mock_detector_server])
        .generation_server(&mock_generation_server)
        .build()
        .await?;

    // Orchestrator request with unary response for generation internal server error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: generation_server_error_input.into(),
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

    // Assertions for generation internal server error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, orchestrator_error_500);

    // Orchestrator request with unary response for detector internal server error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: detector_error_input.into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(
                        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(),
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

    // Assertions for detector internal server error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, orchestrator_error_500);

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: chunker_error_input.into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(
                        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(),
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

    // Assertions for chunker internal server error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, orchestrator_error_500);

    Ok(())
}

// Validates that requests with output detector configured returns detections
#[test(tokio::test)]
async fn output_detector_detections() -> Result<(), anyhow::Error> {
    // Add output detection mock response for output multiple detections
    let expected_detections = [
        ContentAnalysisResponse {
            start: 5,
            end: 18,
            text: "this one does".into(),
            detection: "has_angle_brackets".into(),
            detection_type: "angle_brackets".into(),
            detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into()),
            score: 1.0,
            evidence: None,
            metadata: Metadata::new(),
        },
        ContentAnalysisResponse {
            start: 6,
            end: 20,
            text: "this other one".into(),
            detection: "has_angle_brackets".into(),
            detection_type: "angle_brackets".into(),
            detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into()),
            score: 1.0,
            evidence: None,
            metadata: Metadata::new(),
        },
    ];

    // Add tokenization results response for output detections mock
    let tokenization_results = vec![
        Token {
            start: 0,
            end: 40,
            text: "This sentence does not have a detection.".to_string(),
        },
        Token {
            start: 41,
            end: 61,
            text: "But <this one does>.".to_string(),
        },
        Token {
            start: 62,
            end: 84,
            text: "Also <this other one>.".to_string(),
        },
    ];

    // Add generation mock response for output detections
    let mock_generation_responses = [
        GeneratedTextResult {
            generated_text: "This sentence does not have a detection. But <this one does>.".into(),
            ..Default::default()
        },
        GeneratedTextResult {
            generated_text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
            ..Default::default()
        }
    ];

    let mut generation_mocks = MockSet::new();
    let mut detector_mocks = MockSet::new();
    let mut chunker_mocks = MockSet::new();

    // Add chunker tokenization for output detection mock
    chunker_mocks.mock( |when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: "Generate two sentences, one that does not have an angle brackets detection, and another one that does have.".to_string(),
            });
        then.pb(TokenizationResults {
            results: vec![
                Token {
                    start: 0,
                    end: 102,
                    text: "Generate two sentences, one that does not have an angle brackets detection, and another that does have.".to_string(),
                },
            ],
            token_count: 0,
        });
    });

    // Add chunker tokenization for output multiple detections mock
    chunker_mocks.mock( |when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: "Generate three sentences, one that does not have an angle brackets detection, and another two that does have.".to_string(),
            });
        then.pb(TokenizationResults {
            results: vec![
                Token {
                    start: 0,
                    end: 109,
                    text: "Generate three sentences, one that does not have an angle brackets detection, and another two does have.".to_string(),
                },
            ],
            token_count: 0,
        });
    });

    // Add generation mock for output detection
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID)
            .pb(TextGenerationTaskRequest {
                text: "Generate two sentences, one that does not have angle brackets detection, and another one that does have.".into(),
                ..Default::default()
            });
        then.pb(mock_generation_responses[0].clone());
    });

    // Add generation mock for output multiple detections
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID)
            .pb(TextGenerationTaskRequest {
                text: "Generate three sentences, one that does not have an angle brackets detection, and another two that does have.".into(),
                ..Default::default()
            });
        then.pb(mock_generation_responses[1].clone());
    });

    // Add chunker tokenization for generated text mock for output detection mock
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".to_string(),
            });
        then.pb(TokenizationResults {
            results: vec![
                tokenization_results[0].clone(),
                tokenization_results[1].clone(),
            ],
            token_count: 0,
        });
    });

    // Add chunker tokenization for generated text mock for output multiple detections mock
    chunker_mocks.mock( |when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
            });
        then.pb(TokenizationResults {
            results: tokenization_results.clone(),
            token_count: 0,
        });
    });

    // Add output detection mock for output detection
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection.".into(),
                    "But <this one does>.".into(),
                ],
                detector_params: DetectorParams::new(),
            });
        then.json([vec![], vec![&expected_detections[0]]]);
    });

    // Add output detection mock for output multiple detections
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection.".into(),
                    "But <this one does>.".into(),
                    "Also <this other one>.".into(),
                ],
                detector_params: DetectorParams::new(),
            });
        then.json([
            vec![],
            vec![&expected_detections[0]],
            vec![&expected_detections[1]],
        ]);
    });

    // Configure mock servers
    let mock_generation_server = MockServer::new_grpc("nlp").with_mocks(generation_mocks);
    let mock_detector_server =
        MockServer::new_http(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE).with_mocks(detector_mocks);
    let mock_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE).with_mocks(chunker_mocks);

    // Run test orchestrator server
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([&mock_detector_server])
        .generation_server(&mock_generation_server)
        .build()
        .await?;

    // Orchestrator request with unary response for output detection
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "Generate two sentences, one that does not have angle brackets detection, and another one that does have.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(), DetectorParams::new())])
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions for output detection
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert_eq!(
        results.generated_text,
        Some("This sentence does not have a detection. But <this one does>.".into())
    );
    assert_eq!(
        results.token_classification_results,
        TextGenTokenClassificationResults {
            input: None,
            output: Some(vec![TokenClassificationResult {
                start: 46_u32,
                end: 59_u32,
                word: expected_detections[0].text.clone(),
                entity: expected_detections[0].detection.clone(),
                entity_group: expected_detections[0].detection_type.clone(),
                detector_id: expected_detections[0].detector_id.clone(),
                score: expected_detections[0].score,
                token_count: None
            }])
        }
    );

    // Orchestrator request with unary response for output multiple detections
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "Generate three sentences, one that does not have an angle brackets detection, and another two that does have.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(), DetectorParams::new())])
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions for output multiple detections
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert_eq!(
        results.generated_text,
        Some(
            "This sentence does not have a detection. But <this one does>. Also <this other one>."
                .into()
        )
    );
    assert_eq!(
        results.token_classification_results,
        TextGenTokenClassificationResults {
            input: None,
            output: Some(vec![
                TokenClassificationResult {
                    start: 46_u32,
                    end: 59_u32,
                    word: expected_detections[0].text.clone(),
                    entity: expected_detections[0].detection.clone(),
                    entity_group: expected_detections[0].detection_type.clone(),
                    detector_id: expected_detections[0].detector_id.clone(),
                    score: expected_detections[0].score,
                    token_count: None
                },
                TokenClassificationResult {
                    start: 68_u32,
                    end: 82_u32,
                    word: expected_detections[1].text.clone(),
                    entity: expected_detections[1].detection.clone(),
                    entity_group: expected_detections[1].detection_type.clone(),
                    detector_id: expected_detections[1].detector_id.clone(),
                    score: expected_detections[1].score,
                    token_count: None
                }
            ])
        }
    );

    Ok(())
}

// Validates that requests with output detector configured returns propagated errors
// from detector, chunker and generation server when applicable
#[test(tokio::test)]
async fn output_detector_client_error() -> Result<(), anyhow::Error> {
    // Add 500 expected output detection mock response
    let expected_detector_error = DetectorError {
        code: 500,
        message: "Internal detector error.".into(),
    };

    let orchestrator_error_500 = server::Error {
        code: http::StatusCode::INTERNAL_SERVER_ERROR,
        details: "unexpected error occurred while processing request".into(),
    };

    // Add input for error scenarios
    let chunker_error_input = "This should return a 500 error on chunker";
    let detector_error_input = "This should return a 500 error on detector";
    let generation_server_error_input = "This should return a 500 error on generation";

    let mut generation_mocks = MockSet::new();
    let mut detector_mocks = MockSet::new();
    let mut chunker_mocks = MockSet::new();

    // Add output detection mock
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![generation_server_error_input.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([[[Vec::<ContentAnalysisResponse>::new()]]]);
    });

    // Add output detection mock for detector internal server error scenario
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![detector_error_input.into()],
                detector_params: DetectorParams::new(),
            });
        then.internal_server_error().json(&expected_detector_error);
    });

    // Add generation mock for generation internal server error scenario
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID)
            .pb(TokenizationTaskRequest {
                text: generation_server_error_input.into(),
            });
        then.internal_server_error();
    });

    // Add generation mock for detector internal server error scenario
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID)
            .pb(TokenizationTaskRequest {
                text: detector_error_input.into(),
            });
        then.pb(GeneratedTextResult {
            generated_text: detector_error_input.into(),
            ..Default::default()
        });
    });

    // Add generation mock for chunker internal server error scenario
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID)
            .pb(TokenizationTaskRequest {
                text: chunker_error_input.into(),
            });
        then.pb(GeneratedTextResult {
            generated_text: chunker_error_input.into(),
            ..Default::default()
        });
    });

    // Add chunker tokenization mock for detector internal server error scenario
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
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

    // Add chunker tokenization mock for chunker internal server error scenario
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: chunker_error_input.into(),
            });
        then.internal_server_error();
    });

    // Configure mock servers
    let mock_generation_server = MockServer::new_grpc("nlp").with_mocks(generation_mocks);
    let mock_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE).with_mocks(chunker_mocks);
    let mock_detector_server =
        MockServer::new_http(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE).with_mocks(detector_mocks);

    // Run test orchestrator server
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([&mock_detector_server])
        .generation_server(&mock_generation_server)
        .build()
        .await?;

    // Orchestrator request with unary response for generation internal server error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: generation_server_error_input.into(),
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

    // Assertions for generation internal server error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, orchestrator_error_500);

    // Orchestrator request with unary response for detector internal server error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: detector_error_input.into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(
                        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(),
                        DetectorParams::new(),
                    )]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions for detector internal server error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, orchestrator_error_500);

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: chunker_error_input.into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(
                        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(),
                        DetectorParams::new(),
                    )]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions for chunker internal server error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, orchestrator_error_500);

    Ok(())
}

// Validate that invalid orchestrator requests returns 422 error
#[test(tokio::test)]
async fn orchestrator_validation_error() -> Result<(), anyhow::Error> {
    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .build()
        .await?;

    // Extra request parameter scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&serde_json::json!({
            "model_id": MODEL_ID,
            "inputs": "This request does not comply with the orchestrator API",
            "guardrail_config": {},
            "text_gen_parameters": {},
            "non_existing_field": "random value"
        }))
        .send()
        .await?;

    let results = response.json::<server::Error>().await?;
    debug!("{results:#?}");
    assert_eq!(results.code, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        results
            .details
            .starts_with("non_existing_field: unknown field `non_existing_field`")
    );

    // Invalid input detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 422".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(
                        ANSWER_RELEVANCE_DETECTOR_SENTENCE.into(),
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

    let results = response.json::<server::Error>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        server::Error {
            code: http::StatusCode::UNPROCESSABLE_ENTITY,
            details: format!(
                "detector `{ANSWER_RELEVANCE_DETECTOR_SENTENCE}` is not supported by this endpoint"
            ),
        },
        "failed on input detector with invalid type scenario"
    );

    // non-existing input detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 404".into(),
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

    let results = response.json::<server::Error>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        server::Error {
            code: http::StatusCode::NOT_FOUND,
            details: format!("detector `{NON_EXISTING_DETECTOR}` not found"),
        },
        "failed on non-existing detector scenario"
    );

    // Invalid output detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 422".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(
                        ANSWER_RELEVANCE_DETECTOR_SENTENCE.into(),
                        DetectorParams::new(),
                    )]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    let results = response.json::<server::Error>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        server::Error {
            code: http::StatusCode::UNPROCESSABLE_ENTITY,
            details: format!(
                "detector `{ANSWER_RELEVANCE_DETECTOR_SENTENCE}` is not supported by this endpoint"
            ),
        },
        "failed on output detector with invalid type scenario"
    );

    // non-existing output detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 404".into(),
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

    let results = response.json::<server::Error>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        server::Error {
            code: http::StatusCode::NOT_FOUND,
            details: format!("detector `{NON_EXISTING_DETECTOR}` not found"),
        },
        "failed on non-existing output detector scenario"
    );

    Ok(())
}
