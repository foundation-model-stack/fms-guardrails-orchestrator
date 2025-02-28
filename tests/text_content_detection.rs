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
use mocktail::{prelude::*, utils::find_available_port};
use tracing::debug;

pub mod common;

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
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&TextContentDetectionHttpRequest {
            content: "This sentence has no detections.".into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response);

    // assertions
    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<TextContentDetectionResult>().await?
            == TextContentDetectionResult::default()
    );

    Ok(())
}

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
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        Some(vec![mock_chunker_server]),
    )
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

    debug!(?response);

    assert!(response.status() == StatusCode::OK);
    assert!(
        response.json::<TextContentDetectionResult>().await?
            == TextContentDetectionResult::default()
    );

    Ok(())
}

/// Asserts a scenario with a single detection works as expected (assumes a detector configured with whole_doc_chunker).
///
/// This test mocks a detector that detects text between <angle brackets>.
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
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&TextContentDetectionHttpRequest {
            content: "This sentence has <a detection here>.".into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response);

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

/// Asserts a scenario with a single detection works as expected (with sentence chunker).
///
/// This test mocks a detector that detects text between <angle brackets>.
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
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        Some(vec![mock_chunker_server]),
    )
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

    debug!(?response);

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

#[test(tokio::test)]
async fn test_detector_returns_503() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;
    let expected_detector_error = DetectorError {
        code: 503,
        message: "The detector service is overloaded.".into(),
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 503".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::SERVICE_UNAVAILABLE),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&TextContentDetectionHttpRequest {
            content: "This should return a 503".into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response, "RESPONSE RECEIVED FROM ORCHESTRATOR");

    // assertions
    assert!(response.status() == StatusCode::SERVICE_UNAVAILABLE);

    let response: OrchestratorError = response.json().await?;
    assert!(response.code == 503);
    assert!(
        response.details
            == format!(
                "detector request failed for `{}`: {}",
                detector_name, expected_detector_error.message
            )
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_detector_returns_404() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;
    let expected_detector_error = DetectorError {
        code: 404,
        message: "The detector service was not found.".into(),
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 404".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::NOT_FOUND),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&TextContentDetectionHttpRequest {
            content: "This should return a 404".into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
        })
        .send()
        .await?;

    debug!(?response, "RESPONSE RECEIVED FROM ORCHESTRATOR");

    // assertions
    assert!(response.status() == StatusCode::NOT_FOUND);

    let response: OrchestratorError = response.json().await?;
    assert!(response.code == 404);
    assert!(
        response.details
            == format!(
                "detector request failed for `{}`: {}",
                detector_name, expected_detector_error.message
            )
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_detector_returns_500() -> Result<(), anyhow::Error> {
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

#[test(tokio::test)]
async fn test_detector_returns_non_compliant_message() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a non-compliant message".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&json!({
                "my_detection": "This message does not comply with the expected API"
            })),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&TextContentDetectionHttpRequest {
            content: "This should return a non-compliant message".into(),
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

#[test(tokio::test)]
async fn test_chunker_returns_an_error() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;

    // Add input chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::post(CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This should return a 500".into(),
            })
            .with_headers(chunker_headers),
            MockResponse::empty().with_code(StatusCode::INTERNAL_SERVER_ERROR),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_chunker_server = GrpcMockServer::new(chunker_id, chunker_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        None,
        Some(vec![mock_chunker_server]),
    )
    .await?;

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

#[test(tokio::test)]
async fn test_request_with_extra_fields_returns_422() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        None,
        None,
    )
    .await?;

    // Make orchestrator call
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

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);

    let response: OrchestratorError = response.json().await?;
    debug!("orchestrator json response body:\n{response:#?}");

    assert!(response.code == 422);
    assert!(response.details.contains("unknown field `extra_args`"));

    Ok(())
}

#[test(tokio::test)]
async fn test_request_missing_detectors_field_returns_422() -> Result<(), anyhow::Error> {
    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        None,
        None,
    )
    .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&json!({
            "content": "This sentence has no detections.",
        }))
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);

    let response: OrchestratorError = response.json().await?;
    debug!("orchestrator json response body:\n{response:#?}");

    assert!(response.code == 422);
    assert!(response.details.starts_with("missing field `detectors`"));

    Ok(())
}

#[test(tokio::test)]
async fn test_request_missing_content_field_returns_422() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;
    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        None,
        None,
    )
    .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&json!({
            "detectors": {detector_name: {}},
        }))
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);

    let response: OrchestratorError = response.json().await?;
    debug!("orchestrator json response body:\n{response:#?}");

    assert!(response.code == 422);
    assert!(response.details.starts_with("missing field `content`"));

    Ok(())
}

#[test(tokio::test)]
async fn test_request_with_empty_detectors_field_returns_422() -> Result<(), anyhow::Error> {
    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        None,
        None,
    )
    .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT)
        .json(&json!({
            "content": "This sentence has no detections.",
            "detectors": {},
        }))
        .send()
        .await?;

    debug!("{response:#?}");

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);

    let response: OrchestratorError = response.json().await?;
    debug!("orchestrator json response body:\n{response:#?}");

    assert!(response.code == 422);
    assert!(response.details == "`detectors` is required");

    Ok(())
}
