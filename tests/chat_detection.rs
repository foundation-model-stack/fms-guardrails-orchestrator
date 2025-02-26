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
use serde_json::json;
use std::collections::HashMap;
use test_log::test;

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
use mocktail::{prelude::*, utils::find_available_port};
use tracing::debug;

pub mod common;

#[test(tokio::test)]
async fn test_detection_below_default_threshold_is_not_returned() -> Result<(), anyhow::Error> {
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
    mocks.insert(
        MockPath::new(Method::POST, CHAT_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ChatDetectionRequest {
                messages: messages.clone(),
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![detection.clone()]),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
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
    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<ChatDetectionResult>().await? == ChatDetectionResult { detections: vec![] }
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_detection_above_default_threshold_is_returned() -> Result<(), anyhow::Error> {
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
    mocks.insert(
        MockPath::new(Method::POST, CHAT_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ChatDetectionRequest {
                messages: messages.clone(),
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![detection.clone()]),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
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
    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<ChatDetectionResult>().await?
            == ChatDetectionResult {
                detections: vec![detection]
            }
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_detector_returns_503() -> Result<(), anyhow::Error> {
    let detector_name = PII_DETECTOR;
    let messages = vec![
        Message {
            role: Role::User,
            content: Some(Content::Text("Why is orchestrator returning 503?".into())),
            ..Default::default()
        },
        Message {
            role: Role::Assistant,
            content: Some(Content::Text("Because the detector returned 503.".into())),
            ..Default::default()
        },
    ];
    let detector_error = DetectorError {
        code: 503,
        message: "The detector is overloaded".into(),
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, CHAT_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ChatDetectionRequest {
                messages: messages.clone(),
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&detector_error).with_code(StatusCode::SERVICE_UNAVAILABLE),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
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
    assert!(response.status() == StatusCode::SERVICE_UNAVAILABLE);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert!(response.code == 503);
    assert!(
        response.details
            == format!(
                "detector request failed for `{}`: {}",
                detector_name, detector_error.message
            )
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_detector_returns_404() -> Result<(), anyhow::Error> {
    let detector_name = PII_DETECTOR;
    let messages = vec![
        Message {
            role: Role::User,
            content: Some(Content::Text("Why is orchestrator returning 404?".into())),
            ..Default::default()
        },
        Message {
            role: Role::Assistant,
            content: Some(Content::Text("Because the detector returned 404.".into())),
            ..Default::default()
        },
    ];
    let detector_error = DetectorError {
        code: 404,
        message: "The detector was not found".into(),
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, CHAT_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ChatDetectionRequest {
                messages: messages.clone(),
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&detector_error).with_code(StatusCode::NOT_FOUND),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
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
    assert!(response.status() == StatusCode::NOT_FOUND);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert!(response.code == 404);
    assert!(
        response.details
            == format!(
                "detector request failed for `{}`: {}",
                detector_name, detector_error.message
            )
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_detector_returns_500() -> Result<(), anyhow::Error> {
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
    mocks.insert(
        MockPath::new(Method::POST, CHAT_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ChatDetectionRequest {
                messages: messages.clone(),
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&detector_error).with_code(StatusCode::INTERNAL_SERVER_ERROR),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
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
    assert!(response.status() == StatusCode::INTERNAL_SERVER_ERROR);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert!(response.code == 500);
    assert!(response.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

#[test(tokio::test)]
async fn test_detector_returns_invalid_message() -> Result<(), anyhow::Error> {
    let detector_name = PII_DETECTOR;
    let messages = vec![
        Message {
            role: Role::User,
            content: Some(Content::Text("Why is orchestrator returning 500?".into())),
            ..Default::default()
        },
        Message {
            role: Role::Assistant,
            content: Some(Content::Text(
                "Because something went wrong. Sorry, I can't give more details.".into(),
            )),
            ..Default::default()
        },
    ];

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, CHAT_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ChatDetectionRequest {
                messages: messages.clone(),
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&json!({
                "message": "I won't comply with the detector API."
            }))
            .with_code(StatusCode::OK),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
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
    assert!(response.status() == StatusCode::INTERNAL_SERVER_ERROR);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert!(response.code == 500);
    assert!(response.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}
