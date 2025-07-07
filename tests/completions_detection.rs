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
    chat_completions::COMPLETIONS_ENDPOINT,
    orchestrator::{
        ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT, ORCHESTRATOR_CONFIG_FILE_PATH,
        TestOrchestratorServer,
    },
};
use fms_guardrails_orchestr8::{
    clients::{
        chunker::MODEL_ID_HEADER_NAME as CHUNKER_MODEL_ID_HEADER_NAME,
        detector::{ContentAnalysisRequest, ContentAnalysisResponse},
        openai::{
            ChatCompletion, ChatCompletionChoice, ChatCompletionMessage, ChatDetections,
            Completion, CompletionChoice, Content, ContentPart, ContentType, InputDetectionResult,
            Message, OrchestratorWarning, OutputDetectionResult, Role, Usage,
        },
    },
    models::{
        DetectionWarningReason, DetectorParams, Metadata, UNSUITABLE_INPUT_MESSAGE,
        UNSUITABLE_OUTPUT_MESSAGE,
    },
    orchestrator::common::current_timestamp,
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

use crate::common::detectors::{
    DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, TEXT_CONTENTS_DETECTOR_ENDPOINT,
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
    let mut mock_openai_server = MockServer::new("completions").with_mocks(completion_mocks);

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
    let mut chat_mocks = MockSet::new();

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

    // Add chat completions mock
    chat_mocks.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "model": MODEL_ID,
            "prompt": prompt,
        }));
        then.json(&completions_response);
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detector_mocks);
    let mock_openai_server = MockServer::new("completions").with_mocks(chat_mocks);

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
    let expected_warnings = vec![
        OrchestratorWarning::new(
            DetectionWarningReason::EmptyOutput,
            "Choice of index 0 has no content. Output detection was not executed",
        ),
        OrchestratorWarning::new(
            DetectionWarningReason::EmptyOutput,
            "Choice of index 1 has no content. Output detection was not executed",
        ),
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
    assert_eq!(results.warnings, expected_warnings);
    assert!(results.detections.is_none());

    Ok(())
}
