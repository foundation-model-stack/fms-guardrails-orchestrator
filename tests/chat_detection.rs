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
    detectors::{CHAT_DETECTOR_ENDPOINT, PII_DETECTOR},
    errors::{DetectorError, OrchestratorError},
    orchestrator::{
        TestOrchestratorServer, ORCHESTRATOR_CHAT_DETECTION_ENDPOINT,
        ORCHESTRATOR_CONFIG_FILE_PATH, ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE,
    },
};
use fms_guardrails_orchestr8::{
    clients::{
        detector::ChatDetectionRequest,
        openai::{Content, Message, Role},
    },
    models::{ChatDetectionHttpRequest, ChatDetectionResult, DetectionResult, DetectorParams},
};
use hyper::StatusCode;
use mocktail::prelude::*;
use serde_json::json;
use test_log::test;
use tracing::debug;

pub mod common;

/// Asserts detections below the default threshold are not returned.
#[test(tokio::test)]
async fn no_detections() -> Result<(), anyhow::Error> {
    let detector_name = PII_DETECTOR;
    let messages = vec![
        Message {
            role: Role::User,
            content: Some(Content::Text("Hi there!".into())),
            ..Default::default()
        },
        Message {
            role: Role::Assistant,
            content: Some(Content::Text("Hello!".into())),
            ..Default::default()
        },
    ];
    let detection = DetectionResult {
        detection_type: "pii".into(),
        detection: "is_pii".into(),
        detector_id: Some(detector_name.into()),
        score: 0.01,
        evidence: None,
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.mock(|when, then| {
        when.post()
            .path(CHAT_DETECTOR_ENDPOINT)
            .json(ChatDetectionRequest {
                messages: messages.clone(),
                detector_params: DetectorParams::new(),
            });
        then.json(vec![detection.clone()]);
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new(detector_name).with_mocks(mocks);

    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_DETECTION_ENDPOINT)
        .json(&ChatDetectionHttpRequest {
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
            messages,
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<ChatDetectionResult>().await?,
        ChatDetectionResult::default()
    );

    Ok(())
}

/// Asserts detections above the default threshold are returned.
#[test(tokio::test)]
async fn detections() -> Result<(), anyhow::Error> {
    let detector_name = PII_DETECTOR;
    let messages = vec![
        Message {
            role: Role::User,
            content: Some(Content::Text("What is his cellphone?".into())),
            ..Default::default()
        },
        Message {
            role: Role::Assistant,
            content: Some(Content::Text("It's +1 (123) 123-4567.".into())),
            ..Default::default()
        },
    ];
    let detection = DetectionResult {
        detection_type: "pii".into(),
        detection: "is_pii".into(),
        detector_id: Some(detector_name.into()),
        score: 0.97,
        evidence: None,
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.mock(|when, then| {
        when.post()
            .path(CHAT_DETECTOR_ENDPOINT)
            .json(ChatDetectionRequest {
                messages: messages.clone(),
                detector_params: DetectorParams::new(),
            });
        then.json(vec![detection.clone()]);
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new(detector_name).with_mocks(mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_DETECTION_ENDPOINT)
        .json(&ChatDetectionHttpRequest {
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
            messages,
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<ChatDetectionResult>().await?,
        ChatDetectionResult {
            detections: vec![detection]
        }
    );

    Ok(())
}

/// Asserts error 500 from detectors is propagated.
#[test(tokio::test)]
async fn client_errors() -> Result<(), anyhow::Error> {
    let detector_name = PII_DETECTOR;
    let messages = vec![
        Message {
            role: Role::User,
            content: Some(Content::Text("Why is orchestrator returning 500?".into())),
            ..Default::default()
        },
        Message {
            role: Role::Assistant,
            content: Some(Content::Text("Because the detector returned 500.".into())),
            ..Default::default()
        },
    ];
    let detector_error = DetectorError {
        code: 500,
        message: "The detector had an error".into(),
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.mock(|when, then| {
        when.post()
            .path(CHAT_DETECTOR_ENDPOINT)
            .json(ChatDetectionRequest {
                messages: messages.clone(),
                detector_params: DetectorParams::new(),
            });
        then.json(&detector_error).internal_server_error();
    });

    // Start orchestrator server and its dependencies
    let mock_detector_server = MockServer::new(detector_name).with_mocks(mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_DETECTION_ENDPOINT)
        .json(&ChatDetectionHttpRequest {
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
            messages,
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 500);
    assert_eq!(response.details, ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

/// Asserts orchestrator validation errors.
#[test(tokio::test)]
async fn orchestrator_validation_error() -> Result<(), anyhow::Error> {
    let detector_name = PII_DETECTOR;

    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .build()
        .await?;

    // Asserts requests with extra fields return 422
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
            "messages": [
                {
                  "content": "What is this test asserting?",
                  "role": "user",
                },
                {
                  "content": "It's making sure requests with extra fields are not accepted.",
                  "role": "assistant",
                }
            ],
            "extra_args": true
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("unknown field `extra_args`"));

    // Asserts requests missing `messages` return 422.
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}}
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `messages`"));

    // Asserts requests missing `detectors` return 422.
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_DETECTION_ENDPOINT)
        .json(&json!({
            "messages": [
                {
                  "content": "What is this test asserting?",
                  "role": "user",
                },
                {
                  "content": "It's making sure requests with extra fields are not accepted.",
                  "role": "assistant",
                }
            ],
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `detectors`"));

    // Asserts requests with empty `detectors` return 422.
    let response = orchestrator_server
        .post(ORCHESTRATOR_CHAT_DETECTION_ENDPOINT)
        .json(&json!({
            "messages": [
                {
                  "content": "What is this test asserting?",
                  "role": "user",
                },
                {
                  "content": "It's making sure requests with extra fields are not accepted.",
                  "role": "assistant",
                }
            ],
            "detectors": {}
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("`detectors` is required"));

    Ok(())
}
