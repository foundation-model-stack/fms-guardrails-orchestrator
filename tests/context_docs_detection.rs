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
use test_log::test;

use common::{
    detectors::{CONTEXT_DOC_DETECTOR_ENDPOINT, FACT_CHECKING_DETECTOR},
    orchestrator::{
        TestOrchestratorServer, ORCHESTRATOR_CONFIG_FILE_PATH,
        ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT,
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
