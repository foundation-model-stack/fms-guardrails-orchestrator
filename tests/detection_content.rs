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

use axum_test::TestServer;
use common::{
    chunker::{MockChunkersServiceServer, CHUNKER_UNARY_ENDPOINT},
    util::{create_orchestrator_shared_state, ensure_global_rustls_state},
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
    server::get_app,
};
use hyper::StatusCode;
use mocktail::prelude::*;
use tracing::debug;
use tracing_test::traced_test;

pub mod common;

// Constants
const ENDPOINT_ORCHESTRATOR: &str = "/api/v2/text/detection/content";
const ENDPOINT_DETECTOR: &str = "/api/v1/text/contents";

const DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC: &str = "angle_brackets_detector_whole_doc";
const DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE: &str = "angle_brackets_detector_sentence";

const CHUNKER_NAME_SENTENCE: &str = "sentence_chunker";

/// Asserts a scenario with a single detection works as expected (assumes a detector configured with whole_doc_chunker).
///
/// This test mocks a detector that detects text between <angle brackets>.
#[traced_test]
#[tokio::test]
async fn test_single_detection_whole_doc() -> Result<(), anyhow::Error> {
    ensure_global_rustls_state();
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, ENDPOINT_DETECTOR),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This sentence has <a detection here>.".to_string()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![vec![ContentAnalysisResponse {
                start: 18,
                end: 35,
                text: "a detection here".to_string(),
                detection: "has_angle_brackets".to_string(),
                detection_type: "angle_brackets".to_string(),
                detector_id: Some(detector_name.to_string()),
                score: 1.0,
                evidence: None,
            }]]),
        ),
    );

    // Setup orchestrator and detector servers
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
    let shared_state = create_orchestrator_shared_state(vec![mock_detector_server], vec![]).await?;
    let server = TestServer::new(get_app(shared_state))?;

    // Make orchestrator call
    let response = server
        .post(ENDPOINT_ORCHESTRATOR)
        .json(&TextContentDetectionHttpRequest {
            content: "This sentence has <a detection here>.".to_string(),
            detectors: HashMap::from([(detector_name.to_string(), DetectorParams::new())]),
        })
        .await;

    debug!(?response);

    // assertions
    response.assert_status(StatusCode::OK);
    response.assert_json(&TextContentDetectionResult {
        detections: vec![ContentAnalysisResponse {
            start: 18,
            end: 35,
            text: "a detection here".to_string(),
            detection: "has_angle_brackets".to_string(),
            detection_type: "angle_brackets".to_string(),
            detector_id: Some(detector_name.to_string()),
            score: 1.0,
            evidence: None,
        }],
    });

    Ok(())
}

/// Asserts a scenario with a single detection works as expected (with sentence chunker).
///
/// This test mocks a detector that detects text between <angle brackets>.
#[traced_test]
#[tokio::test]
async fn test_single_detection_sentence_chunker() -> Result<(), anyhow::Error> {
    ensure_global_rustls_state();

    // Add chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::new(Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".to_string(),
            })
            .with_headers(chunker_headers),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 40,
                        text: "This sentence does not have a detection.".to_string(),
                    },
                    Token {
                        start: 41,
                        end: 61,
                        text: "But <this one does>.".to_string(),
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
        MockPath::new(Method::POST, ENDPOINT_DETECTOR),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection.".to_string(),
                    "But <this one does>.".to_string(),
                ],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![
                vec![],
                vec![ContentAnalysisResponse {
                    start: 4,
                    end: 18,
                    text: "this one does".to_string(),
                    detection: "has_angle_brackets".to_string(),
                    detection_type: "angle_brackets".to_string(),
                    detector_id: Some(detector_name.to_string()),
                    score: 1.0,
                    evidence: None,
                }],
            ]),
        ),
    );

    // Start orchestrator, chunker and detector servers.
    let mock_chunker_server = MockChunkersServiceServer::new(chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
    let shared_state = create_orchestrator_shared_state(
        vec![mock_detector_server],
        vec![(chunker_id, mock_chunker_server)],
    )
    .await?;
    let server = TestServer::new(get_app(shared_state))?;

    // Make orchestrator call
    let response = server
        .post(ENDPOINT_ORCHESTRATOR)
        .json(&TextContentDetectionHttpRequest {
            content: "This sentence does not have a detection. But <this one does>.".to_string(),
            detectors: HashMap::from([(detector_name.to_string(), DetectorParams::new())]),
        })
        .await;

    debug!(?response);

    // assertions
    response.assert_status(StatusCode::OK);
    response.assert_json(&TextContentDetectionResult {
        detections: vec![ContentAnalysisResponse {
            start: 45,
            end: 59,
            text: "this one does".to_string(),
            detection: "has_angle_brackets".to_string(),
            detection_type: "angle_brackets".to_string(),
            detector_id: Some(detector_name.to_string()),
            score: 1.0,
            evidence: None,
        }],
    });

    Ok(())
}
