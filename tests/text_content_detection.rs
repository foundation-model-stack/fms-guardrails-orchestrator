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
    chunker::{CHUNKER_NAME_SENTENCE, CHUNKER_UNARY_ENDPOINT},
    detectors::{
        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC,
        TEXT_CONTENTS_DETECTOR_ENDPOINT,
    },
    errors::{DetectorError, OrchestratorError},
    orchestrator::{
        TestOrchestratorServer, ORCHESTRATOR_CONFIG_FILE_PATH,
        ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT, ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE,
    },
};
use fms_guardrails_orchestr8::{
    clients::{
        chunker::MODEL_ID_HEADER_NAME as CHUNKER_MODEL_ID_HEADER_NAME,
        detector::{ContentAnalysisRequest, ContentAnalysisResponse},
    },
    models::{DetectorParams, TextContentDetectionHttpRequest, TextContentDetectionResult},
    pb::{
        caikit::runtime::chunkers::ChunkerTokenizationTaskRequest,
        caikit_data_model::nlp::{Token, TokenizationResults},
    },
};
use hyper::StatusCode;
use mocktail::prelude::*;
use tracing::debug;

pub mod common;

/// Asserts that generated text with no detections is returned (detector configured with whole_doc_chunker).
#[test(tokio::test)]
async fn test_no_detection_whole_doc() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This sentence has no detections.".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![Vec::<ContentAnalysisResponse>::new()]),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&TextContentDetectionHttpRequest {
            content: "This sentence has no detections.".into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<TextContentDetectionResult>().await?
            == TextContentDetectionResult::default()
    );

    Ok(())
}

/// Asserts that generated text with no detections is returned (detector configured with a sentence chunker).
#[test(tokio::test)]
async fn test_no_detection_sentence_chunker() -> Result<(), anyhow::Error> {
    // Add chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::post(CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. Neither does this one.".into(),
            })
            .with_headers(chunker_headers),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 40,
                        text: "This sentence does not have a detection.".into(),
                    },
                    Token {
                        start: 41,
                        end: 64,
                        text: "Neither does this one.".into(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );

    // Add detector mock
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection.".into(),
                    "Neither does this one.".into(),
                ],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![
                Vec::<ContentAnalysisResponse>::new(),
                Vec::<ContentAnalysisResponse>::new(),
            ]),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_chunker_server = GrpcMockServer::new(chunker_id, chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&TextContentDetectionHttpRequest {
            content: "This sentence does not have a detection. Neither does this one.".into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!("{response:#?}");

    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<TextContentDetectionResult>().await?
            == TextContentDetectionResult::default()
    );

    Ok(())
}

/// Asserts that detections are returned (detector configured with whole_doc_chunker).
#[test(tokio::test)]
async fn test_single_detection_whole_doc() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This sentence has <a detection here>.".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![vec![ContentAnalysisResponse {
                start: 18,
                end: 35,
                text: "a detection here".into(),
                detection: "has_angle_brackets".into(),
                detection_type: "angle_brackets".into(),
                detector_id: Some(detector_name.into()),
                score: 1.0,
                evidence: None,
            }]]),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&TextContentDetectionHttpRequest {
            content: "This sentence has <a detection here>.".into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<TextContentDetectionResult>().await?
            == TextContentDetectionResult {
                detections: vec![ContentAnalysisResponse {
                    start: 18,
                    end: 35,
                    text: "a detection here".into(),
                    detection: "has_angle_brackets".into(),
                    detection_type: "angle_brackets".into(),
                    detector_id: Some(detector_name.into()),
                    score: 1.0,
                    evidence: None,
                }],
            }
    );

    Ok(())
}

/// Asserts that detections are returned (detector configured with a sentence chunker).
#[test(tokio::test)]
async fn test_single_detection_sentence_chunker() -> Result<(), anyhow::Error> {
    // Add chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::post(CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".into(),
            })
            .with_headers(chunker_headers),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 40,
                        text: "This sentence does not have a detection.".into(),
                    },
                    Token {
                        start: 41,
                        end: 61,
                        text: "But <this one does>.".into(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );

    // Add detector mock
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection.".into(),
                    "But <this one does>.".into(),
                ],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![
                vec![],
                vec![ContentAnalysisResponse {
                    start: 4,
                    end: 18,
                    text: "this one does".into(),
                    detection: "has_angle_brackets".into(),
                    detection_type: "angle_brackets".into(),
                    detector_id: Some(detector_name.into()),
                    score: 1.0,
                    evidence: None,
                }],
            ]),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_chunker_server = GrpcMockServer::new(chunker_id, chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .chunker_servers([&mock_chunker_server])
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&TextContentDetectionHttpRequest {
            content: "This sentence does not have a detection. But <this one does>.".into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<TextContentDetectionResult>().await?
            == TextContentDetectionResult {
                detections: vec![ContentAnalysisResponse {
                    start: 45,
                    end: 59,
                    text: "this one does".into(),
                    detection: "has_angle_brackets".into(),
                    detection_type: "angle_brackets".into(),
                    detector_id: Some(detector_name.into()),
                    score: 1.0,
                    evidence: None,
                }],
            }
    );

    Ok(())
}

/// Asserts clients returning errors.
#[test(tokio::test)]
async fn client_error() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;
    let expected_detector_error = DetectorError {
        code: 500,
        message: "Internal error on detector call.".into(),
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 500".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error)
                .with_code(StatusCode::INTERNAL_SERVER_ERROR),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&TextContentDetectionHttpRequest {
            content: "This should return a 500".into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response, "RESPONSE RECEIVED FROM ORCHESTRATOR");

    // assertions
    assert!(response.status() == StatusCode::INTERNAL_SERVER_ERROR);

    let response: OrchestratorError = response.json().await?;
    assert!(response.code == 500);
    assert!(response.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

/// Asserts orchestrator validation errors.
#[test(tokio::test)]
async fn orchestrator_validation_error() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .build()
        .await?;

    // assert request with extra fields
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&json!({
            "content": "This sentence has no detections.",
            "detectors": {detector_name: {}},
            "extra_args": true
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response: OrchestratorError = response.json().await?;
    debug!("orchestrator json response body:\n{response:#?}");
    assert!(response.code == 422);
    assert!(response.details.contains("unknown field `extra_args`"));

    // assert request missing `detectors`
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&json!({
            "content": "This sentence has no detections.",
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response: OrchestratorError = response.json().await?;
    debug!("orchestrator json response body:\n{response:#?}");
    assert!(response.code == 422);
    assert!(response.details.starts_with("missing field `detectors`"));

    // assert request missing `content`
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response: OrchestratorError = response.json().await?;
    debug!("orchestrator json response body:\n{response:#?}");
    assert!(response.code == 422);
    assert!(response.details.starts_with("missing field `content`"));

    // assert empty `detectors`
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&json!({
            "content": "This sentence has no detections.",
            "detectors": {},
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let response: OrchestratorError = response.json().await?;
    debug!("orchestrator json response body:\n{response:#?}");
    assert!(response.code == 422);
    assert!(response.details == "`detectors` is required");

    Ok(())
}
