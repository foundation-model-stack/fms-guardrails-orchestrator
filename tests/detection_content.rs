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

use std::{collections::HashMap, sync::Arc};

use axum_test::TestServer;
use common::{ensure_global_rustls_state, CONFIG_FILE_PATH};
use fms_guardrails_orchestr8::{
    clients::detector::{ContentAnalysisRequest, ContentAnalysisResponse},
    config::OrchestratorConfig,
    models::{DetectorParams, TextContentDetectionHttpRequest, TextContentDetectionResult},
    orchestrator::Orchestrator,
    server::{get_app, ServerState},
};
use hyper::StatusCode;
use mocktail::mock::MockSet;
use mocktail::prelude::*;
use tracing::debug;
use tracing_test::traced_test;

mod common;

/// Asserts a scenario with a single detection works as expected (assumes a detector configured with whole_doc_chunker).
///
/// This test mocks a detector that detects text between <angle brackets>.
#[traced_test]
#[tokio::test]
async fn test_single_detection_whole_doc() {
    ensure_global_rustls_state();
    let detector_name = "angle_brackets_detector_whole_doc";

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, "/api/v1/text/contents"),
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

    // Start mock server
    let mock_detector_server = HttpMockServer::new(detector_name, mocks).unwrap();
    let _ = mock_detector_server.start().await;

    let mut config = OrchestratorConfig::load(CONFIG_FILE_PATH).await.unwrap();

    // assign mock server port to detector config
    config
        .detectors
        .get_mut(detector_name)
        .unwrap()
        .service
        .port = Some(mock_detector_server.addr().port());

    let orchestrator = Orchestrator::new(config, false).await.unwrap();
    let shared_state = Arc::new(ServerState::new(orchestrator));
    let server = TestServer::new(get_app(shared_state)).unwrap();

    // Make orchestrator call
    let response = server
        .post("/api/v2/text/detection/content")
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
}
