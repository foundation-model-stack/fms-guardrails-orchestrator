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

use common::{
    chunker::{CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE, CHUNKER_STREAMING_ENDPOINT},
    detectors::{
        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC,
        DETECTOR_NAME_PARENTHESIS_SENTENCE, FACT_CHECKING_DETECTOR_SENTENCE, NON_EXISTING_DETECTOR,
        TEXT_CONTENTS_DETECTOR_ENDPOINT,
    },
    errors::DetectorError,
    orchestrator::{
        ORCHESTRATOR_CONFIG_FILE_PATH, ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT,
        TestOrchestratorServer, json_lines_stream,
    },
};
use fms_guardrails_orchestr8::{
    clients::detector::{ContentAnalysisRequest, ContentAnalysisResponse},
    models::{
        DetectorParams, Metadata, StreamingContentDetectionRequest,
        StreamingContentDetectionResponse,
    },
    pb::{
        caikit::runtime::chunkers::BidiStreamingChunkerTokenizationTaskRequest,
        caikit_data_model::nlp::{ChunkerTokenizationStreamResult, Token},
    },
    server,
};
use futures::StreamExt;
use mocktail::{MockSet, server::MockServer};
use serde_json::json;
use test_log::test;
use tracing::debug;

pub mod common;

/// Asserts scenario with no detections
#[test(tokio::test)]
async fn no_detections() -> Result<(), anyhow::Error> {
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let angle_brackets_detector = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let parenthesis_detector = DETECTOR_NAME_PARENTHESIS_SENTENCE;

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Hi".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " there!".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " How".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " you?".into(),
                    input_index_stream: 4,
                },
            ]);

        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 9,
                    text: "Hi there!".into(),
                }],
                token_count: 0,
                processed_index: 9,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 0,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 9,
                    end: 22,
                    text: " How are you?".into(),
                }],
                token_count: 0,
                processed_index: 22,
                start_index: 9,
                input_start_index: 0,
                input_end_index: 0,
            },
        ]);
    });

    // Add input detection mocks
    let mut detection_mocks = MockSet::new();
    detection_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["Hi there!".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });
    detection_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![" How are you?".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Run test orchestrator server
    let mock_chunker_server = MockServer::new_grpc(chunker_id).with_mocks(chunker_mocks);
    let mock_angle_brackets_detector_server =
        MockServer::new_http(angle_brackets_detector).with_mocks(detection_mocks.clone());
    let mock_parenthesis_detector_server =
        MockServer::new_http(parenthesis_detector).with_mocks(detection_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([
            &mock_angle_brackets_detector_server,
            &mock_parenthesis_detector_server,
        ])
        .chunker_servers([&mock_chunker_server])
        .build()
        .await?;

    // Single-detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: Some(HashMap::from([(
                    angle_brackets_detector.into(),
                    DetectorParams::new(),
                )])),
                content: "Hi".into(),
            },
            StreamingContentDetectionRequest {
                detectors: None,
                content: " there!".into(),
            },
            StreamingContentDetectionRequest {
                detectors: None,
                content: " How".into(),
            },
            StreamingContentDetectionRequest {
                detectors: None,
                content: " are".into(),
            },
            StreamingContentDetectionRequest {
                detectors: None,
                content: " you?".into(),
            },
        ])))
        .send()
        .await?;

    let mut messages = Vec::<StreamingContentDetectionResponse>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }

    let expected_messages = [
        StreamingContentDetectionResponse {
            detections: vec![],
            start_index: 0,
            processed_index: 9,
        },
        StreamingContentDetectionResponse {
            detections: vec![],
            start_index: 9,
            processed_index: 22,
        },
    ];
    assert_eq!(
        messages, expected_messages,
        "failed on single-detector scenario"
    );

    // Multi-detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: Some(HashMap::from([
                    (angle_brackets_detector.into(), DetectorParams::new()),
                    (parenthesis_detector.into(), DetectorParams::new()),
                ])),
                content: "Hi".into(),
            },
            StreamingContentDetectionRequest {
                detectors: None,
                content: " there!".into(),
            },
            StreamingContentDetectionRequest {
                detectors: None,
                content: " How".into(),
            },
            StreamingContentDetectionRequest {
                detectors: None,
                content: " are".into(),
            },
            StreamingContentDetectionRequest {
                detectors: None,
                content: " you?".into(),
            },
        ])))
        .send()
        .await?;

    let mut messages = Vec::<StreamingContentDetectionResponse>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }

    let expected_messages = [
        StreamingContentDetectionResponse {
            detections: vec![],
            start_index: 0,
            processed_index: 9,
        },
        StreamingContentDetectionResponse {
            detections: vec![],
            start_index: 9,
            processed_index: 22,
        },
    ];
    assert_eq!(
        messages, expected_messages,
        "failed on multi-detector scenario"
    );

    Ok(())
}

/// Asserts scenario with detections
#[test(tokio::test)]
async fn detections() -> Result<(), anyhow::Error> {
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let angle_brackets_detector = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let parenthesis_detector = DETECTOR_NAME_PARENTHESIS_SENTENCE;

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb_stream(vec![BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "Hi (there)! How are <you>?".into(),
                input_index_stream: 0,
            }]);

        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 11,
                    text: "Hi (there)!".into(),
                }],
                token_count: 0,
                processed_index: 11,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 0,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 11,
                    end: 26,
                    text: " How are <you>?".into(),
                }],
                token_count: 0,
                processed_index: 26,
                start_index: 11,
                input_start_index: 0,
                input_end_index: 0,
            },
        ]);
    });

    // Add input detection mock
    let mut angle_brackets_detection_mocks = MockSet::new();
    angle_brackets_detection_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["Hi (there)!".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });
    angle_brackets_detection_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![" How are <you>?".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([[ContentAnalysisResponse {
            start: 10,
            end: 13,
            text: "you".into(),
            detection: "has_angle_brackets".into(),
            detection_type: "angle_brackets".into(),
            detector_id: Some(angle_brackets_detector.into()),
            score: 1.0,
            evidence: None,
            metadata: Metadata::new(),
        }]]);
    });

    let mut parenthesis_detection_mocks = MockSet::new();
    parenthesis_detection_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec!["Hi (there)!".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([[ContentAnalysisResponse {
            start: 4,
            end: 9,
            text: "there".into(),
            detection: "has_parenthesis".into(),
            detection_type: "parenthesis".into(),
            detector_id: Some(parenthesis_detector.into()),
            score: 1.0,
            evidence: None,
            metadata: Metadata::new(),
        }]]);
    });
    parenthesis_detection_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![" How are <you>?".into()],
                detector_params: DetectorParams::new(),
            });
        then.json([Vec::<ContentAnalysisResponse>::new()]);
    });

    // Run test orchestrator server
    let mock_chunker_server = MockServer::new_grpc(chunker_id).with_mocks(chunker_mocks);
    let mock_angle_brackets_detector_server =
        MockServer::new_http(angle_brackets_detector).with_mocks(angle_brackets_detection_mocks);
    let mock_parenthesis_detector_server =
        MockServer::new_http(parenthesis_detector).with_mocks(parenthesis_detection_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([
            &mock_angle_brackets_detector_server,
            &mock_parenthesis_detector_server,
        ])
        .chunker_servers([&mock_chunker_server])
        .build()
        .await?;

    // Single-detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: Some(HashMap::from([(
                    angle_brackets_detector.into(),
                    DetectorParams::new(),
                )])),
                content: "Hi (there)! How are <you>?".into(),
            },
        ])))
        .send()
        .await?;

    let mut messages = Vec::<StreamingContentDetectionResponse>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }

    let expected_messages = [
        StreamingContentDetectionResponse {
            detections: vec![],
            start_index: 0,
            processed_index: 11,
        },
        StreamingContentDetectionResponse {
            detections: vec![ContentAnalysisResponse {
                start: 10,
                end: 13,
                text: "you".into(),
                detection: "has_angle_brackets".into(),
                detection_type: "angle_brackets".into(),
                detector_id: Some(angle_brackets_detector.into()),
                score: 1.0,
                evidence: None,
                metadata: Metadata::new(),
            }],
            start_index: 11,
            processed_index: 26,
        },
    ];
    assert_eq!(
        messages, expected_messages,
        "failed on single-detector scenario"
    );

    // Multi-detector scenario
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: Some(HashMap::from([
                    (angle_brackets_detector.into(), DetectorParams::new()),
                    (parenthesis_detector.into(), DetectorParams::new()),
                ])),
                content: "Hi (there)! How are <you>?".into(),
            },
        ])))
        .send()
        .await?;

    let mut messages = Vec::<StreamingContentDetectionResponse>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }

    let expected_messages = [
        StreamingContentDetectionResponse {
            detections: vec![ContentAnalysisResponse {
                start: 4,
                end: 9,
                text: "there".into(),
                detection: "has_parenthesis".into(),
                detection_type: "parenthesis".into(),
                detector_id: Some(parenthesis_detector.into()),
                score: 1.0,
                evidence: None,
                metadata: Metadata::new(),
            }],
            start_index: 0,
            processed_index: 11,
        },
        StreamingContentDetectionResponse {
            detections: vec![ContentAnalysisResponse {
                start: 10,
                end: 13,
                text: "you".into(),
                detection: "has_angle_brackets".into(),
                detection_type: "angle_brackets".into(),
                detector_id: Some(angle_brackets_detector.into()),
                score: 1.0,
                evidence: None,
                metadata: Metadata::new(),
            }],
            start_index: 11,
            processed_index: 26,
        },
    ];
    assert_eq!(
        messages, expected_messages,
        "failed on multi-detector scenario"
    );

    Ok(())
}

/// Asserts clients returning errors.
#[test(tokio::test)]
async fn client_error() -> Result<(), anyhow::Error> {
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;

    let chunker_error_payload = "Chunker should return an error.";
    let detector_error_payload = "Detector should return an error.";

    let orchestrator_error_500 = server::Error {
        code: http::StatusCode::INTERNAL_SERVER_ERROR,
        details: "unexpected error occurred while processing request".into(),
    };

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb_stream(vec![BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: chunker_error_payload.into(),
                input_index_stream: 0,
            }]);
        then.internal_server_error();
    });
    chunker_mocks.mock(|when, then| {
        when.path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id)
            .pb_stream(vec![BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: detector_error_payload.into(),
                input_index_stream: 0,
            }]);
        then.pb_stream([ChunkerTokenizationStreamResult {
            results: vec![Token {
                start: 0,
                end: detector_error_payload.len() as i64,
                text: detector_error_payload.into(),
            }],
            token_count: 0,
            processed_index: detector_error_payload.len() as i64,
            start_index: 0,
            input_start_index: 0,
            input_end_index: 0,
        }]);
    });

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .json(ContentAnalysisRequest {
                contents: vec![detector_error_payload.into()],
                detector_params: DetectorParams::new(),
            });
        then.json(DetectorError {
            code: 500,
            message: "There was an error when running the detection".into(),
        });
    });

    // Run test orchestrator server
    let mock_chunker_server = MockServer::new_grpc(chunker_id).with_mocks(chunker_mocks);
    let mock_detector_server = MockServer::new_http(detector_name).with_mocks(detection_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .detector_servers([&mock_detector_server])
        .chunker_servers([&mock_chunker_server])
        .build()
        .await?;

    // Assert chunker error
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: Some(HashMap::from([(
                    detector_name.into(),
                    DetectorParams::new(),
                )])),
                content: chunker_error_payload.into(),
            },
        ])))
        .send()
        .await?;
    let mut messages = Vec::<server::Error>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }
    let expected_messages = [orchestrator_error_500];
    assert_eq!(messages, expected_messages);

    // Assert detector error
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: Some(HashMap::from([(
                    detector_name.into(),
                    DetectorParams::new(),
                )])),
                content: detector_error_payload.into(),
            },
        ])))
        .send()
        .await?;
    let mut messages = Vec::<server::Error>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }
    assert_eq!(messages, expected_messages);

    Ok(())
}

/// Asserts orchestrator request validation
#[test(tokio::test)]
async fn orchestrator_validation_error() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;

    // Run test orchestrator server
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .build()
        .await?;

    // assert extra argument on request
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([json!( {
            "detectors": {detector_name: {}},
            "content": "Hi there!",
            "extra_arg": true
        })])))
        .send()
        .await?;
    let mut messages = Vec::<server::Error>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].code, 422);
    assert!(messages[0].details.starts_with("unknown field `extra_arg`"));

    // assert missing `detectors` on first frame
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([json!( {
            "detectors": {detector_name: {}}
        })])))
        .send()
        .await?;
    let mut messages = Vec::<server::Error>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].code, 422);
    assert!(messages[0].details.starts_with("missing field `content`"));

    // assert missing `detectors` on first frame
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: None,
                content: "Hi".into(),
            },
        ])))
        .send()
        .await?;
    let mut messages = Vec::<server::Error>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }
    let expected_messages = [server::Error {
        code: http::StatusCode::UNPROCESSABLE_ENTITY,
        details: "`detectors` is required for the first message".into(),
    }];
    assert_eq!(
        messages, expected_messages,
        "failed on missing `detectors` scenario"
    );

    // assert empty `detectors` on first frame
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: Some(HashMap::new()),
                content: "Hi".into(),
            },
        ])))
        .send()
        .await?;
    let mut messages = Vec::<server::Error>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }
    let expected_messages = [server::Error {
        code: http::StatusCode::UNPROCESSABLE_ENTITY,
        details: "`detectors` must not be empty".into(),
    }];
    assert_eq!(
        messages, expected_messages,
        "failed on empty `detectors` scenario"
    );

    // assert detector with invalid type on first frame
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: Some(HashMap::from([(
                    FACT_CHECKING_DETECTOR_SENTENCE.into(),
                    DetectorParams::new(),
                )])),
                content: "Hi".into(),
            },
        ])))
        .send()
        .await?;

    assert_eq!(response.status(), 200);
    let mut messages = Vec::<server::Error>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }

    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        server::Error {
            code: http::StatusCode::UNPROCESSABLE_ENTITY,
            details: format!(
                "detector `{FACT_CHECKING_DETECTOR_SENTENCE}` is not supported by this endpoint"
            )
        },
        "failed at invalid input detector scenario"
    );

    // assert detector with invalid chunker on first frame
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: Some(HashMap::from([(
                    DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(),
                    DetectorParams::new(),
                )])),
                content: "Hi".into(),
            },
        ])))
        .send()
        .await?;

    assert_eq!(response.status(), 200);
    let mut messages = Vec::<server::Error>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }

    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        server::Error {
            code: http::StatusCode::UNPROCESSABLE_ENTITY,
            details: format!(
                "detector `{DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC}` uses chunker `whole_doc_chunker`, which is not supported by this endpoint"
            )
        },
        "failed at detector with invalid chunker scenario"
    );

    // assert non-existing detector on first frame
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT)
        .header("content-type", "application/x-ndjson")
        .body(reqwest::Body::wrap_stream(json_lines_stream([
            StreamingContentDetectionRequest {
                detectors: Some(HashMap::from([(
                    NON_EXISTING_DETECTOR.into(),
                    DetectorParams::new(),
                )])),
                content: "Hi".into(),
            },
        ])))
        .send()
        .await?;

    assert_eq!(response.status(), 200);
    let mut messages = Vec::<server::Error>::with_capacity(1);
    let mut stream = response.bytes_stream();
    while let Some(Ok(msg)) = stream.next().await {
        debug!("recv: {msg:?}");
        messages.push(serde_json::from_slice(&msg[..]).unwrap());
    }

    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        server::Error {
            code: http::StatusCode::NOT_FOUND,
            details: format!("detector `{NON_EXISTING_DETECTOR}` not found")
        },
        "failed at non-existing input detector scenario"
    );

    Ok(())
}
