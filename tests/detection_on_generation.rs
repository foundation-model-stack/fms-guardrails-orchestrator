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
    detectors::{ANSWER_RELEVANCE_DETECTOR, DETECTION_ON_GENERATION_DETECTOR_ENDPOINT},
    errors::{DetectorError, OrchestratorError},
    orchestrator::{
        TestOrchestratorServer, ORCHESTRATOR_CONFIG_FILE_PATH,
        ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT, ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE,
    },
};
use fms_guardrails_orchestr8::{
    clients::detector::GenerationDetectionRequest,
    models::{
        DetectionOnGeneratedHttpRequest, DetectionOnGenerationResult, DetectionResult,
        DetectorParams,
    },
};
use hyper::StatusCode;
use mocktail::{prelude::*, utils::find_available_port};
use tracing::debug;

pub mod common;

#[test(tokio::test)]
async fn test_detection_below_default_threshold_is_not_returned() -> Result<(), anyhow::Error> {
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";
    let generated_text = "The average height of women is 159cm (or 5'3'').";
    let detection = DetectionResult {
        detection_type: "relevance".into(),
        detection: "is_relevant".into(),
        detector_id: Some(detector_name.into()),
        score: 0.49,
        evidence: None,
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, DETECTION_ON_GENERATION_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
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
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&DetectionOnGeneratedHttpRequest {
            prompt: prompt.into(),
            generated_text: generated_text.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response);

    // assertions
    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<DetectionOnGenerationResult>().await?
            == DetectionOnGenerationResult { detections: vec![] }
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_detection_above_default_threshold_is_returned() -> Result<(), anyhow::Error> {
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";
    let generated_text =
        "The average height of men who were born in 1996 was 171cm (or 5'7.5'') in 2014.";
    let detection = DetectionResult {
        detection_type: "relevance".into(),
        detection: "is_relevant".into(),
        detector_id: Some(detector_name.into()),
        score: 0.89,
        evidence: None,
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, DETECTION_ON_GENERATION_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
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
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&DetectionOnGeneratedHttpRequest {
            prompt: prompt.into(),
            generated_text: generated_text.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response);

    // assertions
    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<DetectionOnGenerationResult>().await?
            == DetectionOnGenerationResult {
                detections: vec![detection.clone()]
            }
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_detector_returns_503() -> Result<(), anyhow::Error> {
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";
    let generated_text =
        "The average height of men who were born in 1996 was 171cm (or 5'7.5'') in 2014.";
    let detector_error = DetectorError {
        code: 503,
        message: "The detector is overloaded.".into(),
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, DETECTION_ON_GENERATION_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
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
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&DetectionOnGeneratedHttpRequest {
            prompt: prompt.into(),
            generated_text: generated_text.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response);

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
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";
    let generated_text =
        "The average height of men who were born in 1996 was 171cm (or 5'7.5'') in 2014.";
    let detector_error = DetectorError {
        code: 404,
        message: "The detector is overloaded.".into(),
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, DETECTION_ON_GENERATION_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
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
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&DetectionOnGeneratedHttpRequest {
            prompt: prompt.into(),
            generated_text: generated_text.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response);

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
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";
    let generated_text =
        "The average height of men who were born in 1996 was 171cm (or 5'7.5'') in 2014.";
    let detector_error = DetectorError {
        code: 500,
        message: "The detector is overloaded.".into(),
    };

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, DETECTION_ON_GENERATION_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
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
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&DetectionOnGeneratedHttpRequest {
            prompt: prompt.into(),
            generated_text: generated_text.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response);

    // assertions
    assert!(response.status() == StatusCode::INTERNAL_SERVER_ERROR);
    let response = response.json::<OrchestratorError>().await?;
    assert!(response.code == detector_error.code);
    assert!(response.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

#[test(tokio::test)]
async fn test_detector_returns_non_compliant_message() -> Result<(), anyhow::Error> {
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";
    let generated_text =
        "The average height of men who were born in 1996 was 171cm (or 5'7.5'') in 2014.";

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, DETECTION_ON_GENERATION_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&json!({
                "my_detection": "This message does not comply with the expected API"
            })),
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
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&DetectionOnGeneratedHttpRequest {
            prompt: prompt.into(),
            generated_text: generated_text.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response);

    // assertions
    assert!(response.status() == StatusCode::INTERNAL_SERVER_ERROR);
    let response = response.json::<OrchestratorError>().await?;
    assert!(response.code == 500);
    assert!(response.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

#[test(tokio::test)]
async fn test_request_with_extra_fields_returns_422() -> Result<(), anyhow::Error> {
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";
    let generated_text =
        "The average height of men who were born in 1996 was 171cm (or 5'7.5'') in 2014.";

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
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&json!({
            "prompt": prompt,
            "generated_text": generated_text,
            "detectors": {detector_name: {}},
            "extra_args": true
        }))
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert!(response.code == 422);
    assert!(response.details.contains("unknown field `extra_args`"));

    Ok(())
}
