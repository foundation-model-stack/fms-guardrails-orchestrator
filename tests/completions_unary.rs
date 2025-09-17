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
    openai::COMPLETIONS_ENDPOINT,
    orchestrator::{
        ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT, ORCHESTRATOR_CONFIG_FILE_PATH,
        TestOrchestratorServer,
    },
};
use fms_guardrails_orchestr8::{
    clients::{
        chunker::MODEL_ID_HEADER_NAME as CHUNKER_MODEL_ID_HEADER_NAME,
        detector::{ContentAnalysisRequest, ContentAnalysisResponse, GenerationDetectionRequest},
        openai::{
            Completion, CompletionChoice, CompletionDetectionWarning, CompletionDetections,
            CompletionInputDetections, CompletionOutputDetections, TokenizeResponse, Usage,
        },
    },
    models::{
        DetectionResult, DetectionWarningReason, DetectorParams, UNSUITABLE_INPUT_MESSAGE,
        UNSUITABLE_OUTPUT_MESSAGE,
    },
    orchestrator::{common::current_timestamp, types::Detection},
    pb::{
        caikit::runtime::chunkers::ChunkerTokenizationTaskRequest,
        caikit_data_model::nlp::{Token, TokenizationResults},
    },
    server,
};
use hyper::StatusCode;
use mocktail::prelude::*;
use serde_json::json;
use test_log::test;
use tracing::debug;
use uuid::Uuid;

use crate::common::{
    chunker::CHUNKER_UNARY_ENDPOINT,
    detectors::{
        ANSWER_RELEVANCE_DETECTOR, DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE,
        DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, FACT_CHECKING_DETECTOR, NON_EXISTING_DETECTOR,
        PII_DETECTOR_SENTENCE, TEXT_CONTENTS_DETECTOR_ENDPOINT, TEXT_GENERATION_DETECTOR_ENDPOINT,
    },
    errors::DetectorError,
    openai::TOKENIZE_ENDPOINT,
};

pub mod common;

// Constants
const CHUNKER_NAME_SENTENCE: &str = "sentence_chunker";
const MODEL_ID: &str = "my-super-model-8B";

// Validate passthrough scenario
#[test(tokio::test)]
async fn no_detectors() -> Result<(), anyhow::Error> {
    let prompt = "Hi there!";

    // Add mocksets
    let mut completion_mocks = MockSet::new();

    let expected_choices = vec![
        CompletionChoice {
            index: 0,
            text: "Hi there!".into(),
            logprobs: None,
            finish_reason: Some("length".into()),
            stop_reason: None,
            prompt_logprobs: None,
        },
        CompletionChoice {
            index: 1,
            text: "Hello!".into(),
            logprobs: None,
            finish_reason: Some("length".into()),
            stop_reason: None,
            prompt_logprobs: None,
        },
    ];
    let completions_response = Completion {
        id: Uuid::new_v4().simple().to_string(),
        object: "text_completion".into(),
        created: current_timestamp().as_secs() as i64,
        model: MODEL_ID.into(),
        choices: expected_choices,
        usage: Some(Usage {
            prompt_tokens: 4,
            total_tokens: 36,
            completion_tokens: 32,
            ..Default::default()
        }),
        ..Default::default()
    };

    // Add completions mock
    completion_mocks.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "prompt": prompt,
        }));
        then.json(&completions_response);
    });

    // Start orchestrator server and its dependencies
    let mock_openai_server = MockServer::new_http("openai").with_mocks(completion_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&mock_openai_server)
        .build()
        .await?;

    // Empty `detectors` scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {},
            "prompt": prompt,
        }))
        .send()
        .await?;
    dbg!(&response);

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<Completion>().await?;
    assert_eq!(results.choices[0], completions_response.choices[0]);
    assert_eq!(results.choices[1], completions_response.choices[1]);
    assert_eq!(results.warnings, vec![]);
    assert!(results.detections.is_none());

    // Missing `detectors` scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "prompt": prompt,
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<Completion>().await?;
    assert_eq!(results.choices[0], completions_response.choices[0]);
    assert_eq!(results.choices[1], completions_response.choices[1]);
    assert_eq!(results.warnings, vec![]);
    assert!(results.detections.is_none());

    // `detectors` with empty `input` and `output` scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "prompt": prompt,
            "detectors": {
                "input": {},
                "output": {},
            },
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<Completion>().await?;
    assert_eq!(results.choices[0], completions_response.choices[0]);
    assert_eq!(results.choices[1], completions_response.choices[1]);
    assert_eq!(results.warnings, vec![]);
    assert!(results.detections.is_none());

    Ok(())
}

// Validate that requests without detectors, input detector and output detector configured
// returns text generated by model
#[test(tokio::test)]
async fn no_detections() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    let prompt = "Hi there!";

    // Add mocksets
    let mut detector_mocks = MockSet::new();
    let mut completion_mocks = MockSet::new();

    let expected_choices = vec![
        CompletionChoice {
            index: 0,
            text: "Hi there!".into(),
            logprobs: None,
            finish_reason: Some("length".into()),
            stop_reason: None,
            prompt_logprobs: None,
        },
        CompletionChoice {
            index: 1,
            text: "Hello!".into(),
            logprobs: None,
            finish_reason: Some("length".into()),
            stop_reason: None,
            prompt_logprobs: None,
        },
    ];

    let completions_response = Completion {
        id: Uuid::new_v4().simple().to_string(),
        object: "text_completion".into(),
        created: current_timestamp().as_secs() as i64,
        model: MODEL_ID.into(),
        choices: expected_choices,
        usage: Some(Usage {
            prompt_tokens: 4,
            total_tokens: 36,
            completion_tokens: 32,
            ..Default::default()
        }),
        ..Default::default()
    };

    // Add detector input mock
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["Hi there!".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });
    // Add detector output mock
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["Hello!".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Add completions mock
    completion_mocks.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "prompt": prompt,
        }));
        then.json(&completions_response);
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new_http(detector_name).with_mocks(detector_mocks);
    let mock_openai_server = MockServer::new_http("openai").with_mocks(completion_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .openai_server(&mock_openai_server)
        .build()
        .await?;

    // Make orchestrator call for input/output no detections
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    detector_name: {},
                },
                "output": {
                    detector_name: {},
                },
            },
            "prompt": prompt
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<Completion>().await?;
    assert_eq!(results.choices[0], completions_response.choices[0]);
    assert_eq!(results.choices[1], completions_response.choices[1]);
    assert_eq!(results.warnings, vec![]);
    assert!(results.detections.is_none());

    // Scenario: output detectors on empty choices responses
    let prompt = "Please provide me an empty message";
    let expected_choices = vec![
        CompletionChoice {
            index: 0,
            text: "".into(),
            logprobs: None,
            finish_reason: Some("length".into()),
            stop_reason: None,
            prompt_logprobs: None,
        },
        CompletionChoice {
            index: 1,
            text: "".into(),
            logprobs: None,
            finish_reason: Some("length".into()),
            stop_reason: None,
            prompt_logprobs: None,
        },
    ];
    let completions_response = Completion {
        id: Uuid::new_v4().simple().to_string(),
        object: "text_completion".into(),
        created: current_timestamp().as_secs() as i64,
        model: MODEL_ID.into(),
        choices: expected_choices,
        usage: Some(Usage {
            prompt_tokens: 4,
            total_tokens: 36,
            completion_tokens: 32,
            ..Default::default()
        }),
        ..Default::default()
    };

    mock_openai_server.mocks().mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "prompt": prompt,
        }));
        then.json(&completions_response);
    });

    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "output": {
                    detector_name: {},
                },
            },
            "prompt": prompt
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<Completion>().await?;
    debug!("{}", serde_json::to_string_pretty(&results)?);
    assert_eq!(results.choices[0], completions_response.choices[0]);
    assert_eq!(results.choices[1], completions_response.choices[1]);
    assert_eq!(
        results.warnings,
        vec![
            CompletionDetectionWarning::new(
                DetectionWarningReason::EmptyOutput,
                "Choice of index 0 has no content. Output detection was not executed",
            ),
            CompletionDetectionWarning::new(
                DetectionWarningReason::EmptyOutput,
                "Choice of index 1 has no content. Output detection was not executed",
            ),
        ]
    );
    assert!(results.detections.is_none());

    Ok(())
}

// Validates that requests with input detector configured returns detections
#[test(tokio::test)]
async fn input_detections() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let prompt = "Hi there! Can you help me with <something>?";

    // Add mocksets
    let mut detector_mocks = MockSet::new();
    let mut chunker_mocks = MockSet::new();
    let mut tokenize_mocks = MockSet::new();

    // Add input detection mock response for input detection
    let expected_detections = vec![Detection {
        start: Some(34),
        end: Some(42),
        text: Some("something".into()),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(detector_name.into()),
        score: 1.0,
        ..Default::default()
    }];

    let completions_response = Completion {
        id: Uuid::new_v4().simple().to_string(),
        object: "text_completion".into(),
        created: current_timestamp().as_secs() as i64,
        model: MODEL_ID.into(),
        choices: vec![],
        detections: Some(CompletionDetections {
            input: vec![CompletionInputDetections {
                message_index: 0,
                results: expected_detections.clone(),
            }],
            output: vec![],
        }),
        usage: Some(Usage {
            prompt_tokens: 43,
            ..Default::default()
        }),
        ..Default::default()
    };

    // Add chunker tokenization mock for input detection
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: prompt.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: prompt.len() as i64,
                text: prompt.into(),
            }],
            token_count: 0,
        });
    });

    // Add detector input mock
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![prompt.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([&expected_detections]);
    });

    // Add Tokenize mock
    tokenize_mocks.mock(|when, then| {
        when.post().path(TOKENIZE_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "prompt": prompt,
        }));
        then.json(&TokenizeResponse {
            count: 43,
            ..Default::default()
        });
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new_http(detector_name).with_mocks(detector_mocks);
    let mock_openai_server = MockServer::new_http("openai").with_mocks(tokenize_mocks);
    let mock_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE).with_mocks(chunker_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .openai_server(&mock_openai_server)
        .build()
        .await?;

    // Make orchestrator call for input/output no detections
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    detector_name: {},
                },
                "output": {}
            },
            "prompt": prompt
        }))
        .send()
        .await?;

    // Assertions for input detections
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<Completion>().await?;
    assert_eq!(results.detections, completions_response.detections);
    assert_eq!(results.choices, completions_response.choices);
    assert_eq!(
        results.warnings,
        vec![CompletionDetectionWarning::new(
            DetectionWarningReason::UnsuitableInput,
            UNSUITABLE_INPUT_MESSAGE,
        )]
    );

    Ok(())
}

// Validates that requests with input detector configured returns propagated errors
#[test(tokio::test)]
async fn input_client_error() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    // Add 500 expected input detector mock response
    let expected_detector_error = DetectorError {
        code: 500,
        message: "Internal detector error.".into(),
    };
    // Add 500 expected orchestrator error response
    let expected_orchestrator_error = server::Error {
        code: http::StatusCode::INTERNAL_SERVER_ERROR,
        details: "unexpected error occurred while processing request".into(),
    };

    // Add input for error scenarios
    let chunker_error_input = "This should return a 500 error on chunker";
    let detector_error_input = "This should return a 500 error on detector";
    let completions_error_input = "This should return a 500 error on openai";

    // Add mocksets
    let mut chunker_mocks = MockSet::new();
    let mut detector_mocks = MockSet::new();
    let mut completions_mocks = MockSet::new();

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

    // Add chunker tokenization mock for completions internal server error scenario
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: completions_error_input.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: completions_error_input.len() as i64,
                text: completions_error_input.into(),
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

    // Add detector mock for completions error scenario
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![completions_error_input.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Add detector mock for detector error scenario
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![detector_error_input.into()],
                detector_params: DetectorParams::new(),
            });
        then.internal_server_error().json(&expected_detector_error);
    });

    // Add completions mock for completions error scenario
    completions_mocks.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "prompt": completions_error_input,
        }));
        then.internal_server_error();
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new_http(detector_name).with_mocks(detector_mocks);
    let mock_openai_server = MockServer::new_http("openai").with_mocks(completions_mocks);
    let mock_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE).with_mocks(chunker_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .openai_server(&mock_openai_server)
        .build()
        .await?;

    // Make orchestrator call for chunker error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    detector_name: {},
                },
                "output": {}
            },
            "prompt": chunker_error_input,
        }))
        .send()
        .await?;

    // Assertions for chunker error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, expected_orchestrator_error);

    // Make orchestrator call for detector error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    detector_name: {},
                },
                "output": {}
            },
            "prompt": detector_error_input
        }))
        .send()
        .await?;

    // Assertions for detector error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, expected_orchestrator_error);

    // Make orchestrator call for completions error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    detector_name: {},
                },
                "output": {}
            },
            "prompt": completions_error_input,
        }))
        .send()
        .await?;

    // Assertions for completions error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, expected_orchestrator_error);

    Ok(())
}

// Validates that requests with output detector configured returns detections
#[test(tokio::test)]
async fn output_detections() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let prompt = "Hi there! Can you help me with something?";
    let output_no_detection = "Sure! Let me help you with something, just tell me what you need.";
    let output_with_detection =
        "Sure! Let me help you with <something>, just tell me what you need.";

    // Add mocksets
    let mut detector_mocks = MockSet::new();
    let mut completion_mocks = MockSet::new();
    let mut chunker_mocks = MockSet::new();

    // Add output detection mock response for output detection
    let expected_detections = vec![Detection {
        start: Some(28),
        end: Some(37),
        text: Some("something".into()),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(detector_name.into()),
        score: 1.0,
        ..Default::default()
    }];

    // Add completion choices response for output detection
    let expected_choices = vec![
        CompletionChoice {
            index: 0,
            text: output_no_detection.into(),
            logprobs: None,
            finish_reason: Some("length".into()),
            stop_reason: None,
            prompt_logprobs: None,
        },
        CompletionChoice {
            index: 1,
            text: output_with_detection.into(),
            logprobs: None,
            finish_reason: Some("length".into()),
            stop_reason: None,
            prompt_logprobs: None,
        },
    ];

    let completions_response = Completion {
        id: Uuid::new_v4().simple().to_string(),
        object: "text_completion".into(),
        created: current_timestamp().as_secs() as i64,
        model: MODEL_ID.into(),
        choices: expected_choices,
        detections: Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 1,
                results: expected_detections.clone(),
            }],
        }),
        ..Default::default()
    };

    // Add detector output mock for first message
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![output_no_detection.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Add detector output mock for generated message
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![output_with_detection.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([&expected_detections]);
    });

    // Add chunker tokenization mock for output detection user input
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: prompt.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: prompt.len() as i64,
                text: prompt.into(),
            }],
            token_count: 0,
        });
    });

    // Add chunker tokenization mock for output detection assistant output
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: output_no_detection.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: output_no_detection.len() as i64,
                text: output_no_detection.into(),
            }],
            token_count: 0,
        });
    });
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: output_with_detection.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: output_with_detection.len() as i64,
                text: output_with_detection.into(),
            }],
            token_count: 0,
        });
    });

    // Add completions mock
    completion_mocks.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "prompt": prompt,
        }));
        then.json(&completions_response);
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new_http(detector_name).with_mocks(detector_mocks);
    let mock_openai_server = MockServer::new_http("openai").with_mocks(completion_mocks);
    let mock_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE).with_mocks(chunker_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .openai_server(&mock_openai_server)
        .build()
        .await?;

    // Make orchestrator call for output detections
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    detector_name: {},
                },
            },
            "prompt": prompt,
        }))
        .send()
        .await?;

    // Assertions for output detections
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<Completion>().await?;
    assert_eq!(results.detections, completions_response.detections);
    assert_eq!(results.choices, completions_response.choices);
    assert_eq!(
        results.warnings,
        vec![CompletionDetectionWarning::new(
            DetectionWarningReason::UnsuitableOutput,
            UNSUITABLE_OUTPUT_MESSAGE,
        )]
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_multiple_detector_types() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?"
        }));
        then.json(Completion {
            id: "cmpl-test".into(),
            created: 1749227854,
            model: model_id.into(),
            choices: vec![CompletionChoice {
                index: 0,
                text: "Here are 2 random phone numbers:\n\n1. (503) 272-8192\n2. (617) 985-3519."
                    .into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            }],
            ..Default::default()
        });
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: "Here are 2 random phone numbers:\n\n1. (503) 272-8192\n2. (617) 985-3519."
                    .into(),
            });
        then.pb(TokenizationResults {
            results: vec![
                Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:".into(),
                },
                Token {
                    start: 32,
                    end: 51,
                    text: "\n\n1. (503) 272-8192".into(),
                },
                Token {
                    start: 51,
                    end: 70,
                    text: "\n2. (617) 985-3519.".into(),
                },
            ],
            token_count: 0,
        });
    });

    let mut pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec![
                    "Here are 2 random phone numbers:".into(),
                    "\n\n1. (503) 272-8192".into(),
                    "\n2. (617) 985-3519.".into(),
                ],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
            [],
            [{
                "start": 5,
                "end": 19,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(503) 272-8192",
                "evidences": []
            }],
            [{
                "start": 4,
                "end": 18,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(617) 985-3519",
                "evidences": []
            }]
        ]));
    });

    let mut answer_relevance_detector_server = MockServer::new_http(ANSWER_RELEVANCE_DETECTOR);
    answer_relevance_detector_server.mock(|when, then| {
        when.post()
            .path(TEXT_GENERATION_DETECTOR_ENDPOINT)
            .header("detector-id", ANSWER_RELEVANCE_DETECTOR)
            .json(GenerationDetectionRequest {
                prompt: "Can you generate 2 random phone numbers?".into(),
                generated_text:
                    "Here are 2 random phone numbers:\n\n1. (503) 272-8192\n2. (617) 985-3519."
                        .into(),
                detector_params: DetectorParams::default(),
            });
        then.json(vec![DetectionResult {
            detection_type: "risk".into(),
            detection: "Yes".into(),
            detector_id: Some(ANSWER_RELEVANCE_DETECTOR.into()),
            score: 0.80,
            ..Default::default()
        }]);
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([
            &pii_detector_sentence_server,
            &answer_relevance_detector_server,
        ])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_SENTENCE: {},
                    ANSWER_RELEVANCE_DETECTOR: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?"
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let completion = response.json::<Completion>().await?;

    // Validate number of detections
    assert!(
        completion
            .detections
            .as_ref()
            .is_some_and(|detections| detections
                .output
                .first()
                .is_some_and(|output| output.results.len() == 3)),
        "unexpected number of detections"
    );

    // Validate warnings
    assert_eq!(
        completion.warnings,
        vec![CompletionDetectionWarning::new(
            DetectionWarningReason::UnsuitableOutput,
            UNSUITABLE_OUTPUT_MESSAGE,
        )]
    );

    // Validate detections
    let detections = &completion.detections.unwrap().output[0].results;
    assert!(detections.contains(&Detection {
        detector_id: Some("answer_relevance_detector".into()),
        detection_type: "risk".into(),
        detection: "Yes".into(),
        score: 0.8,
        ..Default::default()
    }));
    assert!(detections.contains(&Detection {
        start: Some(37),
        end: Some(51),
        text: Some("(503) 272-8192".into()),
        detector_id: Some("pii_detector_sentence".into()),
        detection_type: "pii".into(),
        detection: "PhoneNumber".into(),
        score: 0.8,
        ..Default::default()
    }));
    assert!(detections.contains(&Detection {
        start: Some(55),
        end: Some(69),
        text: Some("(617) 985-3519".into()),
        detector_id: Some("pii_detector_sentence".into()),
        detection_type: "pii".into(),
        detection: "PhoneNumber".into(),
        score: 0.8,
        ..Default::default()
    }));

    Ok(())
}

// Validates that requests with output detector configured returns propagated errors
// from detector, chunker and completions server when applicable
#[test(tokio::test)]
async fn output_client_error() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    // Add 500 expected output detector mock response
    let expected_detector_error = DetectorError {
        code: 500,
        message: "Internal detector error.".into(),
    };
    // Add 500 expected orchestrator mock response
    let expected_orchestrator_error = server::Error {
        code: http::StatusCode::INTERNAL_SERVER_ERROR,
        details: "unexpected error occurred while processing request".into(),
    };

    // Add input for error scenarios
    let chunker_error_input = "This should return a 500 error on chunker";
    let detector_error_input = "This should return a 500 error on detector";
    let completions_error_input = "This should return a 500 error on openai";

    // Add mocksets
    let mut chunker_mocks = MockSet::new();
    let mut detector_mocks = MockSet::new();
    let mut completion_mocks = MockSet::new();

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

    // Add chunker tokenization mock for completions internal server error scenario
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: completions_error_input.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: completions_error_input.len() as i64,
                text: completions_error_input.into(),
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

    // Add detector mock for completions error scenario
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![completions_error_input.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Add detector mock for detector error scenario
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![detector_error_input.into()],
                detector_params: DetectorParams::new(),
            });
        then.internal_server_error().json(&expected_detector_error);
    });

    // Add completions mock for chunker error scenario
    completion_mocks.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "prompt": chunker_error_input,
        }));
        then.internal_server_error();
    });

    // Add completions mock for detector error scenario
    completion_mocks.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "prompt": detector_error_input,
        }));
        then.internal_server_error().json(&expected_detector_error);
    });

    // Add completions mock for completions error scenario
    completion_mocks.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "prompt": completions_error_input,
        }));
        then.internal_server_error();
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new_http(detector_name).with_mocks(detector_mocks);
    let mock_openai_server = MockServer::new_http("openai").with_mocks(completion_mocks);
    let mock_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE).with_mocks(chunker_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .openai_server(&mock_openai_server)
        .build()
        .await?;

    // Make orchestrator call for chunker error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    detector_name: {},
                },
            },
            "prompt": chunker_error_input
        }))
        .send()
        .await?;

    // Assertions for chunker error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, expected_orchestrator_error);

    // Make orchestrator call for detector error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    detector_name: {},
                },
            },
            "prompt": detector_error_input,
        }))
        .send()
        .await?;

    // Assertions for detector error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, expected_orchestrator_error);

    // Make orchestrator call for completions error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    detector_name: {},
                },
            },
            "prompt": completions_error_input,
        }))
        .send()
        .await?;

    // Assertions for completions error scenario
    let results = response.json::<server::Error>().await?;
    assert_eq!(results, expected_orchestrator_error);

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

    let prompt = "Hi there!";

    // Invalid input detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    FACT_CHECKING_DETECTOR: {},
                },
                "output": {}
            },
            "prompt": prompt,
        }))
        .send()
        .await?;

    let results = response.json::<server::Error>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        server::Error {
            code: http::StatusCode::UNPROCESSABLE_ENTITY,
            details: format!(
                "detector `{FACT_CHECKING_DETECTOR}` is not supported by this endpoint",
            )
        },
        "failed on invalid input detector scenario"
    );

    // Non-existing input detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    NON_EXISTING_DETECTOR: {},
                },
                "output": {}
            },
            "prompt": prompt,
        }))
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
        "failed on non-existing input detector scenario"
    );

    // Invalid output detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    FACT_CHECKING_DETECTOR: {},
                },
            },
            "prompt": prompt,
        }))
        .send()
        .await?;

    let results = response.json::<server::Error>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        server::Error {
            code: http::StatusCode::UNPROCESSABLE_ENTITY,
            details: format!(
                "detector `{FACT_CHECKING_DETECTOR}` is not supported by this endpoint"
            )
        },
        "failed on invalid output detector scenario"
    );

    // Non-existing output detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    NON_EXISTING_DETECTOR: {},
                }
            },
            "prompt": prompt,
        }))
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
        "failed on non-existing input detector scenario"
    );

    // Empty `model` scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": "",
            "prompt": prompt,
        }))
        .send()
        .await?;

    let results = response.json::<server::Error>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        server::Error {
            code: http::StatusCode::UNPROCESSABLE_ENTITY,
            details: "`model` must not be empty".into()
        }
    );

    Ok(())
}
