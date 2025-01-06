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

use axum_test::TestServer;
use common::{ensure_global_rustls_state, shared_state, ONCE};
use fms_guardrails_orchestr8::{
    clients::detector::{ContentAnalysisRequest, ContentAnalysisResponse},
    models::{DetectorParams, TextContentDetectionResult},
    server::get_app,
};
use hyper::StatusCode;
use mocktail::mock::MockSet;
use mocktail::prelude::*;
use serde_json::json;
use tracing::debug;
use tracing_test::traced_test;

mod common;

/// Asserts a scenario with a single detection works as expected.
/// 
/// This test mocks a detector that detects the word "word" in a given input.
#[traced_test]
#[tokio::test]
async fn test_single_detection() {
    ensure_global_rustls_state();
    let shared_state = ONCE.get_or_init(shared_state).await.clone();
    let server = TestServer::new(get_app(shared_state)).unwrap();
    let detector_name = "content_detector";

    // Add detector mock
    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::new(Method::POST, "/api/v1/text/contents"),
        Mock::new(
            MockRequest::json(json!({
                "contents": ["This sentence has a detection on the last word."],
                "detector_params": {},
            })),
            MockResponse::json(vec![vec![ContentAnalysisResponse {
                start: 42,
                end: 46,
                text: "word".to_string(),
                detection: "word".to_string(),
                detection_type: "word_detection".to_string(),
                score: 1.0,
                evidence: None,
            }]]),
        ),
    );

    let mock_detector_server =
        HttpMockServer::new_with_port("content_detector", mocks, 8001).unwrap();
    let _ = mock_detector_server.start().await;

    let response = server
        .post("/api/v2/text/detection/content")
        .json(&json!({
            "content": "This sentence has a detection on the last word.",
            "detectors": {
                detector_name: {}
            }
        }))
        .await;

    debug!("{:#?}", response);

    response.assert_status(StatusCode::OK);
    response.assert_json(&json!(
        {
            "detections": [
                {
                    "start": 42,
                    "end": 46,
                    "text": "word",
                    "detection": "word",
                    "detection_type": "word_detection",
                    "score": 1.0,
                }
            ]
        }
    ));
}
