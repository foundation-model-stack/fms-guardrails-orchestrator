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
    chat_generation::CHAT_COMPLETIONS_ENDPOINT,
    chunker::CHUNKER_UNARY_ENDPOINT,
    detectors::{
        ANSWER_RELEVANCE_DETECTOR, DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE,
        DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, NON_EXISTING_DETECTOR,
        TEXT_CONTENTS_DETECTOR_ENDPOINT,
    },
    errors::{DetectorError, OrchestratorError},
    orchestrator::{
        ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT, ORCHESTRATOR_CONFIG_FILE_PATH,
        TestOrchestratorServer,
    },
};
use fms_guardrails_orchestr8::{
    clients::{
        chunker::MODEL_ID_HEADER_NAME as CHUNKER_MODEL_ID_HEADER_NAME,
        detector::{ContentAnalysisRequest, ContentAnalysisResponse},
        openai::{
            ChatCompletion, ChatCompletionChoice, ChatCompletionMessage, ChatDetections, Content,
            ContentPart, ContentType, InputDetectionResult, Message, OrchestratorWarning,
            OutputDetectionResult, Role,
        },
    },
    models::{
        DetectionWarningReason, DetectorParams, Metadata, UNSUITABLE_INPUT_MESSAGE,
        UNSUITABLE_OUTPUT_MESSAGE,
    },
    pb::{
        caikit::runtime::chunkers::ChunkerTokenizationTaskRequest,
        caikit_data_model::nlp::{Token, TokenizationResults},
    },
};
use hyper::StatusCode;
use mocktail::prelude::*;
use serde_json::json;
use test_log::test;
use tracing::debug;

pub mod common;

// Constants
const CHUNKER_NAME_SENTENCE: &str = "sentence_chunker";
const MODEL_ID: &str = "my-super-model-8B";

// Validate passthrough scenario
#[test(tokio::test)]
async fn no_detectors() -> Result<(), anyhow::Error> {
    let messages = vec![
        Message {
            content: Some(Content::Text("Hi there!".to_string())),
            role: Role::User,
            ..Default::default()
        },
        Message {
            content: Some(Content::Text("".to_string())),
            role: Role::Assistant,
            ..Default::default()
        },
    ];

    // Add mocksets
    let mut chat_mocks = MockSet::new();

    let expected_choices = vec![
        ChatCompletionChoice {
            message: ChatCompletionMessage {
                role: messages[0].role.clone(),
                content: Some("Hi there!".to_string()),
                refusal: None,
                tool_calls: vec![],
            },
            index: 0,
            logprobs: None,
            finish_reason: "NOT_FINISHED".to_string(),
            stop_reason: None,
        },
        ChatCompletionChoice {
            message: ChatCompletionMessage {
                role: messages[1].role.clone(),
                content: Some("Hello!".to_string()),
                refusal: None,
                tool_calls: vec![],
            },
            index: 1,
            logprobs: None,
            finish_reason: "EOS_TOKEN".to_string(),
            stop_reason: None,
        },
    ];
    let chat_completions_response = ChatCompletion {
        model: MODEL_ID.into(),
        choices: expected_choices.clone(),
        detections: None,
        warnings: vec![],
        ..Default::default()
    };

    // Add chat completions mock
    chat_mocks.mock(|when, then| {
        when.post().path(CHAT_COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "messages": messages,
        }));
        then.json(&chat_completions_response);
    });

    // Start orchestrator server and its dependencies
    let mut mock_chat_completions_server =
        MockServer::new("chat_completions").with_mocks(chat_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .chat_generation_server(&mock_chat_completions_server)
        .build()
        .await?;

    // Empty `detectors` scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {},
            "messages": messages,
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ChatCompletion>().await?;
    assert_eq!(results.choices[0], chat_completions_response.choices[0]);
    assert_eq!(results.choices[1], chat_completions_response.choices[1]);
    assert_eq!(results.warnings, vec![]);
    assert!(results.detections.is_none());

    // Missing `detectors` scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "messages": messages,
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ChatCompletion>().await?;
    assert_eq!(results.choices[0], chat_completions_response.choices[0]);
    assert_eq!(results.choices[1], chat_completions_response.choices[1]);
    assert_eq!(results.warnings, vec![]);
    assert!(results.detections.is_none());

    // `detectors` with empty `input` and `output` scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "messages": messages,
            "detectors": {
                "input": {},
                "output": {},
            },
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ChatCompletion>().await?;
    assert_eq!(results.choices[0], chat_completions_response.choices[0]);
    assert_eq!(results.choices[1], chat_completions_response.choices[1]);
    assert_eq!(results.warnings, vec![]);
    assert!(results.detections.is_none());

    // message content as array, `detectors` with empty `input` and `output` scenario
    let messages = vec![
        Message {
            content: Some(Content::Text("Hi there!".to_string())),
            role: Role::User,
            ..Default::default()
        },
        Message {
            content: Some(Content::Array(vec![
                ContentPart {
                    r#type: ContentType::Text,
                    text: Some("How".into()),
                    image_url: None,
                    refusal: None,
                },
                ContentPart {
                    r#type: ContentType::Text,
                    text: Some("are".into()),
                    image_url: None,
                    refusal: None,
                },
                ContentPart {
                    r#type: ContentType::Text,
                    text: Some("you?".into()),
                    image_url: None,
                    refusal: None,
                },
            ])),
            role: Role::Assistant,
            ..Default::default()
        },
    ];

    // add new mock
    mock_chat_completions_server.mock(|when, then| {
        when.post().path(CHAT_COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "messages": messages,
        }));
        then.json(&chat_completions_response);
    });

    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "messages": messages,
            "detectors": {
                "input": {},
                "output": {},
            },
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ChatCompletion>().await?;
    assert_eq!(results.choices[0], chat_completions_response.choices[0]);
    assert_eq!(results.choices[1], chat_completions_response.choices[1]);
    assert_eq!(results.warnings, vec![]);
    assert!(results.detections.is_none());

    Ok(())
}

// Validate that requests without detectors, input detector and output detector configured
// returns text generated by model
#[test(tokio::test)]
async fn no_detections() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    let messages = vec![
        Message {
            content: Some(Content::Text("Hi there!".to_string())),
            role: Role::User,
            ..Default::default()
        },
        Message {
            content: Some(Content::Text("Hello!".to_string())),
            role: Role::Assistant,
            ..Default::default()
        },
    ];

    // Add mocksets
    let mut detector_mocks = MockSet::new();
    let mut chat_mocks = MockSet::new();

    let expected_choices = vec![
        ChatCompletionChoice {
            message: ChatCompletionMessage {
                role: messages[0].role.clone(),
                content: Some("Hi there!".to_string()),
                refusal: None,
                tool_calls: vec![],
            },
            index: 0,
            logprobs: None,
            finish_reason: "NOT_FINISHED".to_string(),
            stop_reason: None,
        },
        ChatCompletionChoice {
            message: ChatCompletionMessage {
                role: messages[1].role.clone(),
                content: Some("Hello!".to_string()),
                refusal: None,
                tool_calls: vec![],
            },
            index: 1,
            logprobs: None,
            finish_reason: "EOS_TOKEN".to_string(),
            stop_reason: None,
        },
    ];
    let chat_completions_response = ChatCompletion {
        model: MODEL_ID.into(),
        choices: expected_choices.clone(),
        detections: None,
        warnings: vec![],
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

    // Add chat completions mock
    chat_mocks.mock(|when, then| {
        when.post().path(CHAT_COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "messages": messages,
        }));
        then.json(&chat_completions_response);
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detector_mocks);
    let mock_chat_completions_server = MockServer::new("chat_completions").with_mocks(chat_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .chat_generation_server(&mock_chat_completions_server)
        .build()
        .await?;

    // Make orchestrator call for input/output no detections
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
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
            "messages": messages,
        }))
        .send()
        .await?;

    // Assertions for no detections
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ChatCompletion>().await?;
    assert_eq!(results.choices[0], chat_completions_response.choices[0]);
    assert_eq!(results.choices[1], chat_completions_response.choices[1]);
    assert_eq!(results.warnings, vec![]);
    assert!(results.detections.is_none());

    Ok(())
}

// Validates that requests with input detector configured returns detections
#[test(tokio::test)]
async fn input_detections() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let input_text = "Hi there! Can you help me with <something>?";

    let messages = vec![Message {
        content: Some(Content::Text(input_text.to_string())),
        role: Role::User,
        ..Default::default()
    }];

    // Add mocksets
    let mut detector_mocks = MockSet::new();
    let mut chunker_mocks = MockSet::new();
    let mut chat_mocks = MockSet::new();

    // Add input detection mock response for input detection
    let expected_detections = vec![ContentAnalysisResponse {
        start: 34,
        end: 42,
        text: "something".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(detector_name.into()),
        score: 1.0,
        evidence: None,
        metadata: Metadata::new(),
    }];

    let chat_completions_response = ChatCompletion {
        model: MODEL_ID.into(),
        choices: vec![],
        detections: Some(ChatDetections {
            input: vec![InputDetectionResult {
                message_index: 0,
                results: expected_detections.clone(),
            }],
            output: vec![],
        }),
        warnings: vec![OrchestratorWarning::new(
            DetectionWarningReason::UnsuitableInput,
            UNSUITABLE_INPUT_MESSAGE,
        )],
        ..Default::default()
    };

    // Add chunker tokenization mock for input detection
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: input_text.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: input_text.len() as i64,
                text: input_text.into(),
            }],
            token_count: 0,
        });
    });

    // Add detector input mock
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![input_text.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([&expected_detections]);
    });

    // Add chat completions mock
    chat_mocks.mock(|when, then| {
        when.post().path(CHAT_COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "messages": messages,
        }));
        then.json(&chat_completions_response);
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detector_mocks);
    let mock_chat_completions_server = MockServer::new("chat_completions").with_mocks(chat_mocks);
    let mock_chunker_server = MockServer::new(CHUNKER_NAME_SENTENCE)
        .grpc()
        .with_mocks(chunker_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .chat_generation_server(&mock_chat_completions_server)
        .build()
        .await?;

    // Make orchestrator call for input/output no detections
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    detector_name: {},
                },
                "output": {}
            },
            "messages": messages,
        }))
        .send()
        .await?;

    // Assertions for input detections
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ChatCompletion>().await?;
    assert_eq!(results.detections, chat_completions_response.detections);
    assert_eq!(results.choices, chat_completions_response.choices);
    assert_eq!(results.warnings, chat_completions_response.warnings);

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
    let expected_orchestrator_error = OrchestratorError::internal();

    // Add input for error scenarios
    let chunker_error_input = "This should return a 500 error on chunker";
    let detector_error_input = "This should return a 500 error on detector";
    let chat_completions_error_input = "This should return a 500 error on chat completions";

    // Add mocksets
    let mut chunker_mocks = MockSet::new();
    let mut detector_mocks = MockSet::new();
    let mut chat_mocks = MockSet::new();

    let messages_chunker_error = vec![Message {
        content: Some(Content::Text(chunker_error_input.to_string())),
        role: Role::User,
        ..Default::default()
    }];

    let messages_detector_error = vec![Message {
        content: Some(Content::Text(detector_error_input.to_string())),
        role: Role::User,
        ..Default::default()
    }];

    let messages_chat_completions_error = vec![Message {
        content: Some(Content::Text(chat_completions_error_input.to_string())),
        role: Role::User,
        ..Default::default()
    }];

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
                text: chat_completions_error_input.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: chat_completions_error_input.len() as i64,
                text: chat_completions_error_input.into(),
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

    // Add detector mock for chat completions error scenario
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![chat_completions_error_input.into()],
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

    // Add chat completions mock for chat completions error scenario
    chat_mocks.mock(|when, then| {
        when.post().path(CHAT_COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "messages": messages_chat_completions_error,
        }));
        then.internal_server_error();
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detector_mocks);
    let mock_chat_completions_server = MockServer::new("chat_completions").with_mocks(chat_mocks);
    let mock_chunker_server = MockServer::new(CHUNKER_NAME_SENTENCE)
        .grpc()
        .with_mocks(chunker_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .chat_generation_server(&mock_chat_completions_server)
        .build()
        .await?;

    // Make orchestrator call for chunker error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    detector_name: {},
                },
                "output": {}
            },
            "messages": messages_chunker_error,
        }))
        .send()
        .await?;

    // Assertions for chunker error scenario
    let results = response.json::<OrchestratorError>().await?;
    assert_eq!(results, expected_orchestrator_error);

    // Make orchestrator call for detector error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    detector_name: {},
                },
                "output": {}
            },
            "messages": messages_detector_error,
        }))
        .send()
        .await?;

    // Assertions for detector error scenario
    let results = response.json::<OrchestratorError>().await?;
    assert_eq!(results, expected_orchestrator_error);

    // Make orchestrator call for chat completions error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    detector_name: {},
                },
                "output": {}
            },
            "messages": messages_chat_completions_error,
        }))
        .send()
        .await?;

    // Assertions for chat completions error scenario
    let results = response.json::<OrchestratorError>().await?;
    assert_eq!(results, expected_orchestrator_error);

    Ok(())
}

// Validates that requests with output detector configured returns detections
#[test(tokio::test)]
async fn output_detections() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let input_text = "Hi there! Can you help me with something?";
    let output_text = "Sure! Let me help you with <something>, just tell me what you need.";

    let messages = vec![
        Message {
            content: Some(Content::Text(input_text.to_string())),
            role: Role::User,
            ..Default::default()
        },
        Message {
            content: Some(Content::Text(output_text.to_string())),
            role: Role::Assistant,
            ..Default::default()
        },
    ];

    // Add mocksets
    let mut detector_mocks = MockSet::new();
    let mut chat_mocks = MockSet::new();
    let mut chunker_mocks = MockSet::new();

    // Add output detection mock response for output detection
    let expected_detections = vec![ContentAnalysisResponse {
        start: 28,
        end: 37,
        text: "something".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(detector_name.into()),
        score: 1.0,
        evidence: None,
        metadata: Metadata::new(),
    }];

    // Add chat completion choices response for output detection
    let expected_choices = vec![
        ChatCompletionChoice {
            message: ChatCompletionMessage {
                role: messages[0].role.clone(),
                content: Some(input_text.to_string()),
                refusal: None,
                tool_calls: vec![],
            },
            index: 0,
            logprobs: None,
            finish_reason: "NOT_FINISHED".to_string(),
            stop_reason: None,
        },
        ChatCompletionChoice {
            message: ChatCompletionMessage {
                role: messages[1].role.clone(),
                content: Some(output_text.to_string()),
                refusal: None,
                tool_calls: vec![],
            },
            index: 1,
            logprobs: None,
            finish_reason: "EOS_TOKEN".to_string(),
            stop_reason: None,
        },
    ];

    // Add chat completion response for output detection
    let chat_completions_response = ChatCompletion {
        model: MODEL_ID.into(),
        choices: expected_choices.clone(),
        detections: Some(ChatDetections {
            input: vec![],
            output: vec![OutputDetectionResult {
                choice_index: 1,
                results: expected_detections.clone(),
            }],
        }),
        warnings: vec![OrchestratorWarning::new(
            DetectionWarningReason::UnsuitableOutput,
            UNSUITABLE_OUTPUT_MESSAGE,
        )],
        ..Default::default()
    };

    // Add detector output mock for first message
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![input_text.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Add detector output mock for generated message
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![output_text.into()],
                detector_params: DetectorParams::new(),
            });
        then.json([&expected_detections]);
    });

    // Add chunker tokenization mock for output detection user input
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: input_text.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: input_text.len() as i64,
                text: input_text.into(),
            }],
            token_count: 0,
        });
    });

    // Add chunker tokenization mock for output detection assistant output
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest {
                text: output_text.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: output_text.len() as i64,
                text: output_text.into(),
            }],
            token_count: 0,
        });
    });

    // Add chat completions mock
    chat_mocks.mock(|when, then| {
        when.post().path(CHAT_COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "messages": messages,
        }));
        then.json(&chat_completions_response);
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detector_mocks);
    let mock_chat_completions_server = MockServer::new("chat_completions").with_mocks(chat_mocks);
    let mock_chunker_server = MockServer::new(CHUNKER_NAME_SENTENCE)
        .grpc()
        .with_mocks(chunker_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .chat_generation_server(&mock_chat_completions_server)
        .build()
        .await?;

    // Make orchestrator call for output detections
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    detector_name: {},
                },
            },
            "messages": messages,
        }))
        .send()
        .await?;

    // Assertions for output detections
    assert_eq!(response.status(), StatusCode::OK);
    let results = response.json::<ChatCompletion>().await?;
    assert_eq!(results.detections, chat_completions_response.detections);
    assert_eq!(results.choices, chat_completions_response.choices);
    assert_eq!(results.warnings, chat_completions_response.warnings);

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
    let expected_orchestrator_error = OrchestratorError::internal();

    // Add input for error scenarios
    let chunker_error_input = "This should return a 500 error on chunker";
    let detector_error_input = "This should return a 500 error on detector";
    let chat_completions_error_input = "This should return a 500 error on chat completions";

    // Add mocksets
    let mut chunker_mocks = MockSet::new();
    let mut detector_mocks = MockSet::new();
    let mut chat_mocks = MockSet::new();

    let messages_chunker_error = vec![Message {
        content: Some(Content::Text(chunker_error_input.to_string())),
        role: Role::User,
        ..Default::default()
    }];

    let messages_detector_error = vec![Message {
        content: Some(Content::Text(detector_error_input.to_string())),
        role: Role::User,
        ..Default::default()
    }];

    let messages_chat_completions_error = vec![Message {
        content: Some(Content::Text(chat_completions_error_input.to_string())),
        role: Role::User,
        ..Default::default()
    }];

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
                text: chat_completions_error_input.into(),
            });
        then.pb(TokenizationResults {
            results: vec![Token {
                start: 0,
                end: chat_completions_error_input.len() as i64,
                text: chat_completions_error_input.into(),
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

    // Add detector mock for chat completions error scenario
    detector_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![chat_completions_error_input.into()],
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

    // Add chat completions mock for chunker error scenario
    chat_mocks.mock(|when, then| {
        when.post().path(CHAT_COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "messages": messages_chunker_error,
        }));
        then.internal_server_error();
    });

    // Add chat completions mock for detector error scenario
    chat_mocks.mock(|when, then| {
        when.post().path(CHAT_COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "messages": messages_detector_error,
        }));
        then.internal_server_error().json(&expected_detector_error);
    });

    // Add chat completions mock for chat completions error scenario
    chat_mocks.mock(|when, then| {
        when.post().path(CHAT_COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "messages": messages_chat_completions_error,
        }));
        then.internal_server_error();
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detector_mocks);
    let mock_chat_completions_server = MockServer::new("chat_completions").with_mocks(chat_mocks);
    let mock_chunker_server = MockServer::new(CHUNKER_NAME_SENTENCE)
        .grpc()
        .with_mocks(chunker_mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .chat_generation_server(&mock_chat_completions_server)
        .build()
        .await?;

    // Make orchestrator call for chunker error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    detector_name: {},
                },
            },
            "messages": messages_chunker_error,
        }))
        .send()
        .await?;

    // Assertions for chunker error scenario
    let results = response.json::<OrchestratorError>().await?;
    assert_eq!(results, expected_orchestrator_error);

    // Make orchestrator call for detector error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    detector_name: {},
                },
            },
            "messages": messages_detector_error,
        }))
        .send()
        .await?;

    // Assertions for detector error scenario
    let results = response.json::<OrchestratorError>().await?;
    assert_eq!(results, expected_orchestrator_error);

    // Make orchestrator call for chat completions error scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    detector_name: {},
                },
            },
            "messages": messages_chat_completions_error,
        }))
        .send()
        .await?;

    // Assertions for chat completions error scenario
    let results = response.json::<OrchestratorError>().await?;
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

    let messages = vec![Message {
        content: Some(Content::Text("Hi there!".to_string())),
        role: Role::User,
        ..Default::default()
    }];

    // Invalid input detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    ANSWER_RELEVANCE_DETECTOR: {},
                },
                "output": {}
            },
            "messages": messages,
        }))
        .send()
        .await?;

    let results = response.json::<OrchestratorError>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        OrchestratorError::detector_not_supported(ANSWER_RELEVANCE_DETECTOR),
        "failed on invalid input detector scenario"
    );

    // Non-existing input detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    NON_EXISTING_DETECTOR: {},
                },
                "output": {}
            },
            "messages": messages,
        }))
        .send()
        .await?;

    let results = response.json::<OrchestratorError>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        OrchestratorError::detector_not_found(NON_EXISTING_DETECTOR),
        "failed on non-existing input detector scenario"
    );

    // Invalid output detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    ANSWER_RELEVANCE_DETECTOR: {},
                },
            },
            "messages": messages,
        }))
        .send()
        .await?;

    let results = response.json::<OrchestratorError>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        OrchestratorError::detector_not_supported(ANSWER_RELEVANCE_DETECTOR),
        "failed on invalid output detector scenario"
    );

    // Non-existing output detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {},
                "output": {
                    NON_EXISTING_DETECTOR: {},
                }
            },
            "messages": messages,
        }))
        .send()
        .await?;

    let results = response.json::<OrchestratorError>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        OrchestratorError::detector_not_found(NON_EXISTING_DETECTOR),
        "failed on non-existing input detector scenario"
    );

    // input detectors and last message without `content` scenario
    let no_content_messages = vec![
        Message {
            content: Some(Content::Text("Hi there!".to_string())),
            role: Role::User,
            ..Default::default()
        },
        Message {
            role: Role::User,
            ..Default::default()
        },
    ];

    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC: {}

                }
            },
            "messages": no_content_messages,
        }))
        .send()
        .await?;

    let results = response.json::<OrchestratorError>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        OrchestratorError {
            code: 422,
            details: "if input detectors are provided, `content` must not be empty on last message"
                .into()
        }
    );

    // input detectors and last message with empty string as `content` scenario
    let no_content_messages = vec![
        Message {
            content: Some(Content::Text("Hi there!".to_string())),
            role: Role::User,
            ..Default::default()
        },
        Message {
            content: Some(Content::Text("".into())),
            role: Role::User,
            ..Default::default()
        },
    ];

    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC: {}

                }
            },
            "messages": no_content_messages,
        }))
        .send()
        .await?;

    let results = response.json::<OrchestratorError>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        OrchestratorError {
            code: 422,
            details: "if input detectors are provided, `content` must not be empty on last message"
                .into()
        }
    );

    // input detectors and last message with empty array as `content` scenario
    let no_content_messages = vec![
        Message {
            content: Some(Content::Text("Hi there!".to_string())),
            role: Role::User,
            ..Default::default()
        },
        Message {
            content: Some(Content::Array(Vec::new())),
            role: Role::User,
            ..Default::default()
        },
    ];

    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC: {}

                }
            },
            "messages": no_content_messages,
        }))
        .send()
        .await?;

    let results = response.json::<OrchestratorError>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        OrchestratorError {
            code: 422,
            details: "Detection on array is not supported".into()
        }
    );

    // input detectors and last message with array of empty strings as `content` scenario
    let no_content_messages = vec![
        Message {
            content: Some(Content::Text("Hi there!".to_string())),
            role: Role::User,
            ..Default::default()
        },
        Message {
            content: Some(Content::Array(vec![
                ContentPart {
                    r#type: ContentType::Text,
                    text: Some("".into()),
                    image_url: None,
                    refusal: None,
                },
                ContentPart {
                    r#type: ContentType::Text,
                    text: Some("".into()),
                    image_url: None,
                    refusal: None,
                },
            ])),
            role: Role::User,
            ..Default::default()
        },
    ];

    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "model": MODEL_ID,
            "detectors": {
                "input": {
                    DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC: {}

                }
            },
            "messages": no_content_messages,
        }))
        .send()
        .await?;

    let results = response.json::<OrchestratorError>().await?;
    debug!("{results:#?}");
    assert_eq!(
        results,
        OrchestratorError {
            code: 422,
            details: "Detection on array is not supported".into()
        }
    );

    Ok(())
}
