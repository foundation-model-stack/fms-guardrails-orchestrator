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
    detectors::{CONTEXT_DOC_DETECTOR_ENDPOINT, FACT_CHECKING_DETECTOR},
    errors::{DetectorError, OrchestratorError},
    orchestrator::{
        TestOrchestratorServer, ORCHESTRATOR_CONFIG_FILE_PATH,
        ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT, ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE,
    },
};
use fms_guardrails_orchestr8::{
    clients::detector::{ContextDocsDetectionRequest, ContextType},
    models::{ContextDocsHttpRequest, ContextDocsResult, DetectionResult, DetectorParams},
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
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has decreased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];
    let detection = DetectionResult {
        detection_type: "fact_check".into(),
        detection: "is_accurate".into(),
        detector_id: Some(detector_name.into()),
        score: 0.23,
        evidence: None,
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.mock(|when, then| {
        when.post()
            .path(CONTEXT_DOC_DETECTOR_ENDPOINT)
            .json(ContextDocsDetectionRequest {
                detector_params: DetectorParams::new(),
                content: content.into(),
                context_type: ContextType::Url,
                context: context.clone(),
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
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&ContextDocsHttpRequest {
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
            content: content.into(),
            context_type: ContextType::Url,
            context,
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<ContextDocsResult>().await?,
        ContextDocsResult::default()
    );

    Ok(())
}

/// Asserts detections above the default threshold are returned.
#[test(tokio::test)]
async fn detections() -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];
    let detection = DetectionResult {
        detection_type: "fact_check".into(),
        detection: "is_accurate".into(),
        detector_id: Some(detector_name.into()),
        score: 0.91,
        evidence: None,
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.mock(|when, then| {
        when.post()
            .path(CONTEXT_DOC_DETECTOR_ENDPOINT)
            .json(ContextDocsDetectionRequest {
                detector_params: DetectorParams::new(),
                content: content.into(),
                context_type: ContextType::Url,
                context: context.clone(),
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
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&ContextDocsHttpRequest {
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
            content: content.into(),
            context_type: ContextType::Url,
            context,
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<ContextDocsResult>().await?,
        ContextDocsResult {
            detections: vec![detection]
        }
    );

    Ok(())
}

/// Asserts clients returning errors.
#[test(tokio::test)]
async fn client_error() -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];
    let detector_error = DetectorError {
        code: 500,
        message: "The detector is overloaded".into(),
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.mock(|when, then| {
        when.post()
            .path(CONTEXT_DOC_DETECTOR_ENDPOINT)
            .json(ContextDocsDetectionRequest {
                detector_params: DetectorParams::new(),
                content: content.into(),
                context_type: ContextType::Url,
                context: context.clone(),
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
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&ContextDocsHttpRequest {
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
            content: content.into(),
            context_type: ContextType::Url,
            context,
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let response = response.json::<OrchestratorError>().await?;
    assert_eq!(response.code, detector_error.code);
    assert_eq!(response.details, ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

/// Asserts orchestrator validation errors.
#[test(tokio::test)]
async fn orchestrator_validation_error() -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];

    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .build()
        .await?;

    // Asserts requests with extra fields return 422
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
            "content": content,
            "context_type": "url",
            "context": context,
            "extra_args": true
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    assert_eq!(response.code, 422);
    assert!(response.details.contains("unknown field `extra_args`"));

    // Asserts requests missing `content` return 422.
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
            "context_type": "url",
            "context": context,
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `content`"));

    // Asserts requests missing `context` return 422.
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
            "context_type": "url",
            "content": content,
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `context`"));

    // Asserts requests missing `context_type` return 422.
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
            "context": context,
            "content": content,
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `context_type`"));

    // Asserts requests with invalid `context_type` return 422.
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
            "content": content,
            "context": context,
            "context_type": "thoughts"
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response
        .details
        .starts_with("context_type: unknown variant `thoughts`, expected `docs` or `url`"));

    // Asserts requests missing `detectors` return 422.
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "content": content,
            "context": context,
            "context_type": "docs"
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.starts_with("missing field `detectors`"));

    // Asserts requests with empty `detectors` return 422.
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "content": content,
            "context": context,
            "context_type": "docs"
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.starts_with("missing field `detectors`"));

    Ok(())
}
