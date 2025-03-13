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
use mocktail::prelude::*;
use tracing::debug;

pub mod common;

/// Asserts detections below the default threshold are not returned.
#[test(tokio::test)]
async fn no_detections() -> Result<(), anyhow::Error> {
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
    mocks.mock(|when, then| {
        when.post()
            .path(DETECTION_ON_GENERATION_DETECTOR_ENDPOINT)
            .json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
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
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&DetectionOnGeneratedHttpRequest {
            prompt: prompt.into(),
            generated_text: generated_text.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<DetectionOnGenerationResult>().await?,
        DetectionOnGenerationResult::default()
    );

    Ok(())
}

/// Asserts detections above the default threshold are returned.
#[test(tokio::test)]
async fn detections() -> Result<(), anyhow::Error> {
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
    mocks.mock(|when, then| {
        when.post()
            .path(DETECTION_ON_GENERATION_DETECTOR_ENDPOINT)
            .json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
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
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&DetectionOnGeneratedHttpRequest {
            prompt: prompt.into(),
            generated_text: generated_text.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;
    debug!("{response:#?}");

    // assertions
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<DetectionOnGenerationResult>().await?,
        DetectionOnGenerationResult {
            detections: vec![detection]
        }
    );

    Ok(())
}

/// Asserts clients returning errors.
#[test(tokio::test)]
async fn client_error() -> Result<(), anyhow::Error> {
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
    mocks.mock(|when, then| {
        when.post()
            .path(DETECTION_ON_GENERATION_DETECTOR_ENDPOINT)
            .json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
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
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&DetectionOnGeneratedHttpRequest {
            prompt: prompt.into(),
            generated_text: generated_text.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
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
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";
    let generated_text =
        "The average height of men who were born in 1996 was 171cm (or 5'7.5'') in 2014.";

    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .build()
        .await?;

    // asserts request with extra fields
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

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("unknown field `extra_args`"));

    // asserts requests missing `prompt`
    let response = orchestrator_server
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&json!({
            "generated_text": generated_text,
            "detectors": {detector_name: {}},
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `prompt`"));

    // asserts requests missing `generated_text`
    let response = orchestrator_server
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&json!({
            "prompt": prompt,
            "detectors": {detector_name: {}},
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `generated_text`"));

    // asserts requests missing `detectors`
    let response = orchestrator_server
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&json!({
            "prompt": prompt,
            "generated_text": generated_text,
        }))
        .send()
        .await?;

    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `detectors`"));

    // asserts requests with empty `detectors`
    let response = orchestrator_server
        .post(ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT)
        .json(&DetectionOnGeneratedHttpRequest {
            prompt: prompt.into(),
            generated_text: generated_text.into(),
            detectors: HashMap::new(),
        })
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert_eq!(response.details, "`detectors` is required");

    Ok(())
}
