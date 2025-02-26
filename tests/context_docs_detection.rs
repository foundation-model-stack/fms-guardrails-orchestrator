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
use mocktail::{prelude::*, utils::find_available_port};
use tracing::debug;

pub mod common;

#[test(tokio::test)]
async fn test_detection_below_default_threshold_is_not_returned() -> Result<(), anyhow::Error> {
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
    mocks.insert(
        MockPath::new(Method::POST, CONTEXT_DOC_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContextDocsDetectionRequest {
                detector_params: DetectorParams::new(),
                content: content.into(),
                context_type: ContextType::Url,
                context: context.clone(),
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
    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<ContextDocsResult>().await? == ContextDocsResult { detections: vec![] }
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_detection_above_default_threshold_is_returned() -> Result<(), anyhow::Error> {
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
    mocks.insert(
        MockPath::new(Method::POST, CONTEXT_DOC_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContextDocsDetectionRequest {
                detector_params: DetectorParams::new(),
                content: content.into(),
                context_type: ContextType::Url,
                context: context.clone(),
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
    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<ContextDocsResult>().await?
            == ContextDocsResult {
                detections: vec![detection]
            }
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_detector_returns_503() -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];
    let detector_error = DetectorError {
        code: 503,
        message: "The detector is overloaded".into(),
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, CONTEXT_DOC_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContextDocsDetectionRequest {
                detector_params: DetectorParams::new(),
                content: content.into(),
                context_type: ContextType::Url,
                context: context.clone(),
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
    assert!(response.status() == StatusCode::SERVICE_UNAVAILABLE);
    let response = response.json::<OrchestratorError>().await?;
    assert!(response.code == detector_error.code);
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
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];
    let detector_error = DetectorError {
        code: 404,
        message: "The detector is overloaded".into(),
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, CONTEXT_DOC_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContextDocsDetectionRequest {
                detector_params: DetectorParams::new(),
                content: content.into(),
                context_type: ContextType::Url,
                context: context.clone(),
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
    assert!(response.status() == StatusCode::NOT_FOUND);
    let response = response.json::<OrchestratorError>().await?;
    assert!(response.code == detector_error.code);
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
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];
    let detector_error = DetectorError {
        code: 500,
        message: "The detector is overloaded".into(),
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, CONTEXT_DOC_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContextDocsDetectionRequest {
                detector_params: DetectorParams::new(),
                content: content.into(),
                context_type: ContextType::Url,
                context: context.clone(),
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
    assert!(response.status() == StatusCode::INTERNAL_SERVER_ERROR);
    let response = response.json::<OrchestratorError>().await?;
    assert!(response.code == detector_error.code);
    assert!(response.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

#[test(tokio::test)]
async fn test_detector_returns_invalid_response() -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, CONTEXT_DOC_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContextDocsDetectionRequest {
                detector_params: DetectorParams::new(),
                content: content.into(),
                context_type: ContextType::Url,
                context: context.clone(),
            }),
            MockResponse::json(
                &json!({"message": "This response does not comply with the Detector API"}),
            )
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
    assert!(response.status() == StatusCode::INTERNAL_SERVER_ERROR);
    let response = response.json::<OrchestratorError>().await?;
    assert!(response.code == 500);
    assert!(response.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

#[test(tokio::test)]
async fn test_orchestrator_receives_a_request_with_extra_fields() -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, MockSet::new())?;
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

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    assert!(response.code == 422);
    assert!(response.details.contains("unknown field `extra_args`"));

    Ok(())
}

#[test(tokio::test)]
async fn test_orchestrator_receives_a_request_missing_content() -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let context = vec!["https://ourworldindata.org/human-height".to_string()];

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, MockSet::new())?;
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
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
            "context_type": "url",
            "context": context,
        }))
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    assert!(response.code == 422);
    assert!(response.details.contains("missing field `content`"));

    Ok(())
}

#[test(tokio::test)]
async fn test_orchestrator_receives_a_request_missing_context() -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, MockSet::new())?;
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
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
            "context_type": "url",
            "content": content,
        }))
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    assert!(response.code == 422);
    assert!(response.details.contains("missing field `context`"));

    Ok(())
}

#[test(tokio::test)]
async fn test_orchestrator_receives_a_request_missing_context_type() -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, MockSet::new())?;
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
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
            "context": context,
            "content": content,
        }))
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    assert!(response.code == 422);
    assert!(response.details.contains("missing field `context_type`"));

    Ok(())
}

#[test(tokio::test)]
async fn test_orchestrator_receives_a_request_with_invalid_context_type(
) -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, MockSet::new())?;
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

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert!(response.code == 422);
    assert!(response
        .details
        .starts_with("context_type: unknown variant `thoughts`, expected `docs` or `url`"));

    Ok(())
}

#[test(tokio::test)]
async fn test_orchestrator_receives_a_request_missing_detectors() -> Result<(), anyhow::Error> {
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, MockSet::new())?;
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
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "content": content,
            "context": context,
            "context_type": "docs"
        }))
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert!(response.code == 422);
    assert!(response.details.starts_with("missing field `detectors`"));

    Ok(())
}
#[test(tokio::test)]
async fn test_orchestrator_receives_a_request_with_invalid_detectors() -> Result<(), anyhow::Error>
{
    let detector_name = FACT_CHECKING_DETECTOR;
    let content = "The average human height has increased in the past century.";
    let context = vec!["https://ourworldindata.org/human-height".to_string()];

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, MockSet::new())?;
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
        .post(ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT)
        .json(&json!({
            "content": content,
            "context": context,
            "context_type": "docs"
        }))
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert!(response.code == 422);
    assert!(response.details.starts_with("missing field `detectors`"));

    Ok(())
}
