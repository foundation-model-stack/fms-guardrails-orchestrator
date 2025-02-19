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
    chunker::{MockChunkersServiceServer, CHUNKER_NAME_SENTENCE, CHUNKER_UNARY_ENDPOINT},
    detectors::{
        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC,
        TEXT_CONTENTS_DETECTOR_ENDPOINT,
    },
    orchestrator::{
        TestOrchestratorServer, ORCHESTRATOR_CONFIG_FILE_PATH,
        ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT,
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

/// Asserts a scenario with a single detection works as expected (assumes a detector configured with whole_doc_chunker).
///
/// This test mocks a detector that detects text between <angle brackets>.
#[test(tokio::test)]
async fn test_single_detection_whole_doc() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
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
        MockPath::new(Method::POST, CHUNKER_UNARY_ENDPOINT),
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
        MockPath::new(Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
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
    let mock_chunker_server = MockChunkersServiceServer::new(chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(detector_name, mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        Some(vec![(chunker_id.into(), mock_chunker_server)]),
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
