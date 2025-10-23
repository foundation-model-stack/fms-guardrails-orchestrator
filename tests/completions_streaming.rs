pub mod common;

use std::collections::HashMap;

use fms_guardrails_orchestr8::{
    clients::{
        detector::{ContentAnalysisRequest, GenerationDetectionRequest},
        openai::{
            Completion, CompletionChoice, CompletionDetections, CompletionInputDetections,
            CompletionLogprobs, CompletionOutputDetections, ErrorResponse, TokenizeResponse, Usage,
        },
    },
    models::{DetectionResult, DetectorParams},
    orchestrator::types::Detection,
    pb::{
        caikit::runtime::chunkers::{
            BidiStreamingChunkerTokenizationTaskRequest, ChunkerTokenizationTaskRequest,
        },
        caikit_data_model::nlp::{ChunkerTokenizationStreamResult, Token, TokenizationResults},
    },
};
use futures::{StreamExt, TryStreamExt};
use mocktail::prelude::*;
use serde_json::json;
use test_log::test;
use tracing::debug;

use crate::common::{
    chunker::{
        CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE, CHUNKER_STREAMING_ENDPOINT,
        CHUNKER_UNARY_ENDPOINT,
    },
    detectors::{
        ANSWER_RELEVANCE_DETECTOR, PII_DETECTOR_SENTENCE, PII_DETECTOR_WHOLE_DOC,
        TEXT_CONTENTS_DETECTOR_ENDPOINT, TEXT_GENERATION_DETECTOR_ENDPOINT,
    },
    openai::{COMPLETIONS_ENDPOINT, TOKENIZE_ENDPOINT},
    orchestrator::{
        ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT, ORCHESTRATOR_CONFIG_FILE_PATH, SseStream,
        TestOrchestratorServer,
    },
    sse,
};

#[test(tokio::test)]
async fn no_detectors() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");

    let completions_chunks = [
        Completion {
            id: "cmpl-test".into(),
            model: model_id.to_string(),
            created: 1754506038,
            choices: vec![CompletionChoice {
                index: 0,
                text: "I".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
        Completion {
            id: "cmpl-test".into(),
            model: model_id.to_string(),
            created: 1754506038,
            choices: vec![CompletionChoice {
                index: 0,
                text: " am".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
        Completion {
            id: "cmpl-test".into(),
            model: model_id.to_string(),
            created: 1754506038,
            choices: vec![CompletionChoice {
                index: 0,
                text: " great!".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            }],
            ..Default::default()
        },
    ];

    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Hi there! How are you?"
        }));
        then.text_stream(sse(completions_chunks.clone()));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "prompt": "Hi there! How are you?",
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0].choices[0].text,
        completions_chunks[0].choices[0].text
    );
    assert_eq!(
        messages[1].choices[0].text,
        completions_chunks[1].choices[0].text
    );
    assert_eq!(
        messages[2].choices[0].text,
        completions_chunks[2].choices[0].text
    );
    assert_eq!(
        messages[2].choices[0].finish_reason,
        completions_chunks[2].choices[0].finish_reason
    );

    Ok(())
}

#[test(tokio::test)]
async fn no_detectors_n2() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");

    let completions_chunks = [
        Completion {
            id: "cmpl-test".into(),
            model: model_id.to_string(),
            created: 1754506038,
            choices: vec![CompletionChoice {
                index: 0,
                text: "I".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
        Completion {
            id: "cmpl-test".into(),
            model: model_id.to_string(),
            created: 1754506038,
            choices: vec![CompletionChoice {
                index: 1,
                text: "Awesome!".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            }],
            ..Default::default()
        },
        Completion {
            id: "cmpl-test".into(),
            model: model_id.to_string(),
            created: 1754506038,
            choices: vec![CompletionChoice {
                index: 0,
                text: " am".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
        Completion {
            id: "cmpl-test".into(),
            model: model_id.to_string(),
            created: 1754506038,
            choices: vec![CompletionChoice {
                index: 0,
                text: " great!".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            }],
            ..Default::default()
        },
    ];

    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Hi there! How are you?",
            "n": 2
        }));
        then.text_stream(sse(completions_chunks.clone()));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "prompt": "Hi there! How are you?",
            "n": 2
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 4);

    // Validate content messages for both choices
    assert_eq!(
        messages[0].choices[0].text,
        completions_chunks[0].choices[0].text
    );
    assert_eq!(
        messages[1].choices[0].text,
        completions_chunks[1].choices[0].text
    );
    assert_eq!(
        messages[2].choices[0].text,
        completions_chunks[2].choices[0].text
    );
    assert_eq!(
        messages[3].choices[0].text,
        completions_chunks[3].choices[0].text
    );

    // Validate stop messages for both choices
    assert_eq!(
        messages[1].choices[0].finish_reason,
        completions_chunks[1].choices[0].finish_reason
    );
    assert_eq!(
        messages[3].choices[0].finish_reason,
        completions_chunks[3].choices[0].finish_reason
    );

    Ok(())
}

#[test(tokio::test)]
async fn input_detectors() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(TOKENIZE_ENDPOINT)
            .json(json!({
            "model": model_id,
            "prompt": "Here is my social security number: 123-45-6789. Can you generate another one like it?",
        }));
        then.json(&TokenizeResponse {
            count: 24,
            ..Default::default()
        });
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb(ChunkerTokenizationTaskRequest { text: "Here is my social security number: 123-45-6789. Can you generate another one like it?".into() });
        then.pb(TokenizationResults {
            results: vec![
                Token { start: 0, end: 47, text: "Here is my social security number: 123-45-6789.".into() },
                Token { start: 48, end: 85, text: "Can you generate another one like it?".into() },
            ],
            token_count: 0
        });
    });

    let mut pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec![
                    "Here is my social security number: 123-45-6789.".into(),
                    "Can you generate another one like it?".into(),
                ],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 35,
                "end": 46,
                "detection": "NationalNumber.SocialSecurityNumber.US",
                "detection_type": "pii",
                "score": 0.8,
                "text": "123-45-6789",
                "evidences": []
            }
        ]]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_sentence_server])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {
                    PII_DETECTOR_SENTENCE: {}
                },
                "output": {}
            },
            "prompt": "Here is my social security number: 123-45-6789. Can you generate another one like it?",            
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;

    // Validate length
    assert_eq!(messages.len(), 1, "unexpected number of messages");

    // Validate input detections
    assert_eq!(
        messages[0].detections,
        Some(CompletionDetections {
            input: vec![CompletionInputDetections {
                message_index: 0,
                results: vec![Detection {
                    start: Some(35),
                    end: Some(46),
                    text: Some("123-45-6789".into()),
                    detection: "NationalNumber.SocialSecurityNumber.US".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
            output: vec![],
        }),
        "unexpected input detections"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?"
        }));
        then.text_stream(sse([
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "Here".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " are".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " ".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " random".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " phone".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " numbers".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: ":\n\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "1. (503) 272-8192\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2. (617) 985-3519.".into(),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Here".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " ".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " random".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " phone".into(),
                    input_index_stream: 5,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " numbers".into(),
                    input_index_stream: 6,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: ":\n\n".into(),
                    input_index_stream: 7,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "1. (503) 272-8192\n".into(),
                    input_index_stream: 8,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2. (617) 985-3519.".into(),
                    input_index_stream: 9,
                },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:".into(),
                }],
                token_count: 0,
                processed_index: 32,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 7,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 32,
                    end: 51,
                    text: "\n\n1. (503) 272-8192".into(),
                }],
                token_count: 0,
                processed_index: 51,
                start_index: 32,
                input_start_index: 8,
                input_end_index: 8,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 51,
                    end: 70,
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 9,
                input_end_index: 9,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["Here are 2 random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n\n1. (503) 272-8192".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 5,
                "end": 19,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(503) 272-8192",
                "evidences": []
            }
        ]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n2. (617) 985-3519.".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 4,
                "end": 18,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(617) 985-3519",
                "evidences": []
            }
        ]]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_sentence_server])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_SENTENCE: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?"
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 3, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![CompletionChoice {
            index: 0,
            text: "Here are 2 random phone numbers:".into(),
            ..Default::default()
        }],
        "unexpected choices for msg-0"
    );
    // Validate msg-0 detections
    assert_eq!(
        messages[0].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![],
            }],
        }),
        "unexpected detections for msg-0"
    );

    // Validate msg-1 choices
    assert_eq!(
        messages[1].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n\n1. (503) 272-8192".into(),
            ..Default::default()
        }],
        "unexpected choices for msg-1"
    );
    // Validate msg-2 detections
    assert_eq!(
        messages[1].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(5),
                    end: Some(19),
                    text: Some("(503) 272-8192".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-1"
    );

    // Validate msg-2 choices
    assert_eq!(
        messages[2].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n2. (617) 985-3519.".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        }],
        "unexpected choices for msg-2"
    );

    // Validate msg-2 detections
    assert_eq!(
        messages[2].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(4),
                    end: Some(18),
                    text: Some("(617) 985-3519".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-2"
    );

    // Validate finish reason
    assert_eq!(
        messages[2].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_multiple_detector_types() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?"
        }));
        then.text_stream(sse([
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "Here".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " are".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " ".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " random".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " phone".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " numbers".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: ":\n\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "1. (503) 272-8192\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2. (617) 985-3519.".into(),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Here".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " ".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " random".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " phone".into(),
                    input_index_stream: 5,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " numbers".into(),
                    input_index_stream: 6,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: ":\n\n".into(),
                    input_index_stream: 7,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "1. (503) 272-8192\n".into(),
                    input_index_stream: 8,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2. (617) 985-3519.".into(),
                    input_index_stream: 9,
                },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:".into(),
                }],
                token_count: 0,
                processed_index: 32,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 7,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 32,
                    end: 51,
                    text: "\n\n1. (503) 272-8192".into(),
                }],
                token_count: 0,
                processed_index: 51,
                start_index: 32,
                input_start_index: 8,
                input_end_index: 8,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 51,
                    end: 70,
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 9,
                input_end_index: 9,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["Here are 2 random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n\n1. (503) 272-8192".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 5,
                "end": 19,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(503) 272-8192",
                "evidences": []
            }
        ]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n2. (617) 985-3519.".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 4,
                "end": 18,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(617) 985-3519",
                "evidences": []
            }
        ]]));
    });

    let mut answer_relevance_detector_server = MockServer::new_http(ANSWER_RELEVANCE_DETECTOR);
    answer_relevance_detector_server.mock(|when, then| {
        when.post()
            .path(TEXT_GENERATION_DETECTOR_ENDPOINT)
            .header("detector-id", ANSWER_RELEVANCE_DETECTOR)
            .json(GenerationDetectionRequest {
                prompt: "Can you generate 2 random phone numbers?".into(),
                generated_text:
                    "Here are 2 random phone numbers:\n\n1. (503) 272-8192\n2. (617) 985-3519."
                        .into(),
                detector_params: DetectorParams::default(),
            });
        then.json(vec![DetectionResult {
            detection_type: "risk".into(),
            detection: "Yes".into(),
            detector_id: Some(ANSWER_RELEVANCE_DETECTOR.into()),
            score: 0.80,
            ..Default::default()
        }]);
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([
            &pii_detector_sentence_server,
            &answer_relevance_detector_server,
        ])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_SENTENCE: {},
                    ANSWER_RELEVANCE_DETECTOR: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?"
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 4, "unexpected number of messages");

    // Validate chunk detections
    // Validate msg-0
    assert_eq!(
        messages[0].choices,
        vec![CompletionChoice {
            index: 0,
            text: "Here are 2 random phone numbers:".into(),
            ..Default::default()
        }],
        "unexpected choices for msg-0"
    );
    assert_eq!(
        messages[0].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![],
            }],
        }),
        "unexpected detections for msg-0"
    );
    assert!(
        messages[0].warnings.is_empty(),
        "unexpected warnings for msg-0"
    );

    // Validate msg-1
    assert_eq!(
        messages[1].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n\n1. (503) 272-8192".into(),
            ..Default::default()
        }],
        "unexpected choices for msg-1"
    );
    assert_eq!(
        messages[1].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(5),
                    end: Some(19),
                    text: Some("(503) 272-8192".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-1"
    );
    assert_eq!(
        messages[1].warnings.len(),
        1,
        "unexpected warnings for msg-1"
    );

    // Validate msg-2
    assert_eq!(
        messages[2].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n2. (617) 985-3519.".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        }],
        "unexpected choices for msg-2"
    );
    assert_eq!(
        messages[2].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(4),
                    end: Some(18),
                    text: Some("(617) 985-3519".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-2"
    );
    assert_eq!(
        messages[2].warnings.len(),
        1,
        "unexpected warnings for msg-2"
    );
    // Validate finish reason
    assert_eq!(
        messages[2].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason"
    );

    // Validate whole doc detections (text_generation)
    let last = messages.last().unwrap();
    assert_eq!(
        last.detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    detector_id: Some(ANSWER_RELEVANCE_DETECTOR.into()),
                    detection: "Yes".into(),
                    detection_type: "risk".into(),
                    score: 0.8,
                    ..Default::default()
                },],
            }],
        }),
        "unexpected whole doc detections message"
    );
    assert!(
        last.warnings.is_empty(),
        "unexpected warnings for whole doc detections message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_with_logprobs() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?",
            "logprobs": 1
        }));
        then.text_stream(sse([
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "Here".into(),
                    logprobs: Some(CompletionLogprobs {
                        tokens: vec!["Here".into()],
                        token_logprobs: vec![-0.81],
                        top_logprobs: vec![HashMap::from([("Here".into(), -0.81)])],
                        text_offset: vec![0],
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " are".into(),
                    logprobs: Some(CompletionLogprobs {
                        tokens: vec![" are".into()],
                        token_logprobs: vec![-0.81],
                        top_logprobs: vec![HashMap::from([(" are".into(), -0.81)])],
                        text_offset: vec![4],
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " ".into(),
                    logprobs: Some(CompletionLogprobs {
                        tokens: vec![" ".into()],
                        token_logprobs: vec![-0.81],
                        top_logprobs: vec![HashMap::from([(" ".into(), -0.81)])],
                        text_offset: vec![8],
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2".into(),
                    logprobs: Some(CompletionLogprobs {
                        tokens: vec!["2".into()],
                        token_logprobs: vec![-0.81],
                        top_logprobs: vec![HashMap::from([("2".into(), -0.81)])],
                        text_offset: vec![9],
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " random".into(),
                    logprobs: Some(CompletionLogprobs {
                        tokens: vec![" random".into()],
                        token_logprobs: vec![-0.81],
                        top_logprobs: vec![HashMap::from([(" random".into(), -0.81)])],
                        text_offset: vec![10],
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " phone".into(),
                    logprobs: Some(CompletionLogprobs {
                        tokens: vec![" phone".into()],
                        token_logprobs: vec![-0.81],
                        top_logprobs: vec![HashMap::from([(" phone".into(), -0.81)])],
                        text_offset: vec![17],
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " numbers".into(),
                    logprobs: Some(CompletionLogprobs {
                        tokens: vec![" numbers".into()],
                        token_logprobs: vec![-0.81],
                        top_logprobs: vec![HashMap::from([(" numbers".into(), -0.81)])],
                        text_offset: vec![23],
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: ":\n\n".into(),
                    logprobs: Some(CompletionLogprobs {
                        tokens: vec![":\n\n".into()],
                        token_logprobs: vec![-0.81],
                        top_logprobs: vec![HashMap::from([(":\n\n".into(), -0.81)])],
                        text_offset: vec![31],
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "1. (503) 272-8192\n".into(),
                    logprobs: Some(CompletionLogprobs {
                        tokens: vec!["1. (503) 272-8192\n".into()],
                        token_logprobs: vec![-0.81],
                        top_logprobs: vec![HashMap::from([("1. (503) 272-8192\n".into(), -0.81)])],
                        text_offset: vec![34],
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2. (617) 985-3519.".into(),
                    logprobs: Some(CompletionLogprobs {
                        tokens: vec!["2. (617) 985-3519.".into()],
                        token_logprobs: vec![-0.81],
                        top_logprobs: vec![HashMap::from([("2. (617) 985-3519.".into(), -0.81)])],
                        text_offset: vec![52],
                    }),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Here".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " ".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " random".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " phone".into(),
                    input_index_stream: 5,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " numbers".into(),
                    input_index_stream: 6,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: ":\n\n".into(),
                    input_index_stream: 7,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "1. (503) 272-8192\n".into(),
                    input_index_stream: 8,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2. (617) 985-3519.".into(),
                    input_index_stream: 9,
                },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:".into(),
                }],
                token_count: 0,
                processed_index: 32,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 7,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 32,
                    end: 51,
                    text: "\n\n1. (503) 272-8192".into(),
                }],
                token_count: 0,
                processed_index: 51,
                start_index: 32,
                input_start_index: 8,
                input_end_index: 8,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 51,
                    end: 70,
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 9,
                input_end_index: 9,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["Here are 2 random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n\n1. (503) 272-8192".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 5,
                "end": 19,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(503) 272-8192",
                "evidences": []
            }
        ]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n2. (617) 985-3519.".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 4,
                "end": 18,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(617) 985-3519",
                "evidences": []
            }
        ]]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_sentence_server])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "logprobs": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_SENTENCE: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?",
            "logprobs": 1
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 3, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![CompletionChoice {
            index: 0,
            text: "Here are 2 random phone numbers:".into(),
            logprobs: Some(CompletionLogprobs {
                tokens: vec![
                    "Here".into(),
                    " are".into(),
                    " ".into(),
                    "2".into(),
                    " random".into(),
                    " phone".into(),
                    " numbers".into(),
                    ":\n\n".into(),
                ],
                token_logprobs: vec![-0.81, -0.81, -0.81, -0.81, -0.81, -0.81, -0.81, -0.81,],
                top_logprobs: vec![
                    HashMap::from([("Here".into(), -0.81)]),
                    HashMap::from([(" are".into(), -0.81)]),
                    HashMap::from([(" ".into(), -0.81)]),
                    HashMap::from([("2".into(), -0.81)]),
                    HashMap::from([(" random".into(), -0.81)]),
                    HashMap::from([(" phone".into(), -0.81)]),
                    HashMap::from([(" numbers".into(), -0.81)]),
                    HashMap::from([(":\n\n".into(), -0.81)]),
                ],
                text_offset: vec![0, 4, 8, 9, 10, 17, 23, 31,],
            }),
            ..Default::default()
        }],
        "unexpected choices for msg-0"
    );
    // Validate msg-0 detections
    assert_eq!(
        messages[0].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![],
            }],
        }),
        "unexpected detections for msg-0"
    );

    // Validate msg-1 choices
    assert_eq!(
        messages[1].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n\n1. (503) 272-8192".into(),
            logprobs: Some(CompletionLogprobs {
                tokens: vec!["1. (503) 272-8192\n".into(),],
                token_logprobs: vec![-0.81],
                top_logprobs: vec![HashMap::from([("1. (503) 272-8192\n".into(), -0.81)]),],
                text_offset: vec![34],
            }),
            ..Default::default()
        }],
        "unexpected choices for msg-1"
    );
    // Validate msg-1 detections
    assert_eq!(
        messages[1].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(5),
                    end: Some(19),
                    text: Some("(503) 272-8192".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-1"
    );

    // Validate msg-2 choices and stop reason
    assert_eq!(
        messages[2].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n2. (617) 985-3519.".into(),
            logprobs: Some(CompletionLogprobs {
                tokens: vec!["2. (617) 985-3519.".into(),],
                token_logprobs: vec![-0.81],
                top_logprobs: vec![HashMap::from([("2. (617) 985-3519.".into(), -0.81)]),],
                text_offset: vec![52],
            }),
            finish_reason: Some("stop".into()),
            ..Default::default()
        }],
        "unexpected choices for msg-2"
    );
    // Validate msg-2 detections
    assert_eq!(
        messages[2].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(4),
                    end: Some(18),
                    text: Some("(617) 985-3519".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-2"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_with_usage() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?",
            "stream_options": {
                "include_usage": true
            }
        }));
        then.text_stream(sse([
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "Here".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " are".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " ".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " random".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " phone".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " numbers".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: ":\n\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "1. (503) 272-8192\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2. (617) 985-3519.".into(),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 41,
                    completion_tokens: 31,
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Here".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " ".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " random".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " phone".into(),
                    input_index_stream: 5,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " numbers".into(),
                    input_index_stream: 6,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: ":\n\n".into(),
                    input_index_stream: 7,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "1. (503) 272-8192\n".into(),
                    input_index_stream: 8,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2. (617) 985-3519.".into(),
                    input_index_stream: 9,
                },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:".into(),
                }],
                token_count: 0,
                processed_index: 32,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 7,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 32,
                    end: 51,
                    text: "\n\n1. (503) 272-8192".into(),
                }],
                token_count: 0,
                processed_index: 51,
                start_index: 32,
                input_start_index: 8,
                input_end_index: 8,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 51,
                    end: 70,
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 9,
                input_end_index: 9,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["Here are 2 random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n\n1. (503) 272-8192".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 5,
                "end": 19,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(503) 272-8192",
                "evidences": []
            }
        ]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n2. (617) 985-3519.".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 4,
                "end": 18,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(617) 985-3519",
                "evidences": []
            }
        ]]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_sentence_server])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_SENTENCE: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?",
            "stream_options": {
                "include_usage": true
            }
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 3, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![CompletionChoice {
            index: 0,
            text: "Here are 2 random phone numbers:".into(),
            ..Default::default()
        }],
        "unexpected choices for msg-0"
    );
    // Validate msg-0 detections
    assert_eq!(
        messages[0].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![],
            }],
        }),
        "unexpected detections for msg-0"
    );

    // Validate msg-1 choices
    assert_eq!(
        messages[1].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n\n1. (503) 272-8192".into(),
            ..Default::default()
        }],
        "unexpected choices for msg-1"
    );
    // Validate msg-2 detections
    assert_eq!(
        messages[1].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(5),
                    end: Some(19),
                    text: Some("(503) 272-8192".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-1"
    );

    // Validate msg-2 choices
    assert_eq!(
        messages[2].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n2. (617) 985-3519.".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        }],
        "unexpected choices for msg-2"
    );

    // Validate msg-2 detections
    assert_eq!(
        messages[2].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(4),
                    end: Some(18),
                    text: Some("(617) 985-3519".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-2"
    );

    // Validate finish reason
    assert_eq!(
        messages[2].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason"
    );

    // Validate final usage message
    assert_eq!(
        messages[2].usage,
        Some(Usage {
            prompt_tokens: 10,
            total_tokens: 41,
            completion_tokens: 31,
            ..Default::default()
        }),
        "unexpected usage for final usage message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_with_continuous_usage_stats() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?",
            "stream_options": {
                "include_usage": true,
                "continuous_usage_stats": true
            }
        }));
        then.text_stream(sse([
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "Here".into(),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 12,
                    completion_tokens: 2,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " are".into(),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 13,
                    completion_tokens: 3,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " ".into(),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 14,
                    completion_tokens: 4,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2".into(),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 15,
                    completion_tokens: 5,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " random".into(),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 16,
                    completion_tokens: 6,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " phone".into(),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 17,
                    completion_tokens: 7,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " numbers".into(),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 18,
                    completion_tokens: 8,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: ":\n\n".into(),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 19,
                    completion_tokens: 9,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "1. (503) 272-8192\n".into(),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 20,
                    completion_tokens: 10,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2. (617) 985-3519.".into(),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    total_tokens: 21,
                    completion_tokens: 11,
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Here".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " ".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " random".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " phone".into(),
                    input_index_stream: 5,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " numbers".into(),
                    input_index_stream: 6,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: ":\n\n".into(),
                    input_index_stream: 7,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "1. (503) 272-8192\n".into(),
                    input_index_stream: 8,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2. (617) 985-3519.".into(),
                    input_index_stream: 9,
                },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:".into(),
                }],
                token_count: 0,
                processed_index: 32,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 7,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 32,
                    end: 51,
                    text: "\n\n1. (503) 272-8192".into(),
                }],
                token_count: 0,
                processed_index: 51,
                start_index: 32,
                input_start_index: 8,
                input_end_index: 8,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 51,
                    end: 70,
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 9,
                input_end_index: 9,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["Here are 2 random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n\n1. (503) 272-8192".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 5,
                "end": 19,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(503) 272-8192",
                "evidences": []
            }
        ]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n2. (617) 985-3519.".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 4,
                "end": 18,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(617) 985-3519",
                "evidences": []
            }
        ]]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_sentence_server])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_SENTENCE: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?",
            "stream_options": {
                "include_usage": true,
                "continuous_usage_stats": true
            }
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 3, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![CompletionChoice {
            index: 0,
            text: "Here are 2 random phone numbers:".into(),
            ..Default::default()
        }],
        "unexpected choices for msg-0"
    );
    // Validate msg-0 detections
    assert_eq!(
        messages[0].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![],
            }],
        }),
        "unexpected detections for msg-0"
    );
    // Validate msg-0 usage
    assert_eq!(
        messages[0].usage,
        Some(Usage {
            prompt_tokens: 10,
            total_tokens: 19,
            completion_tokens: 9,
            ..Default::default()
        }),
        "unexpected usage for msg-0"
    );

    // Validate msg-1 choices
    assert_eq!(
        messages[1].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n\n1. (503) 272-8192".into(),
            ..Default::default()
        }],
        "unexpected choices for msg-1"
    );
    // Validate msg-1 detections
    assert_eq!(
        messages[1].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(5),
                    end: Some(19),
                    text: Some("(503) 272-8192".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-1"
    );
    // Validate msg-1 usage
    assert_eq!(
        messages[1].usage,
        Some(Usage {
            prompt_tokens: 10,
            total_tokens: 20,
            completion_tokens: 10,
            ..Default::default()
        }),
        "unexpected usage for msg-1"
    );

    // Validate msg-2 choices
    assert_eq!(
        messages[2].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n2. (617) 985-3519.".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        }],
        "unexpected choices for msg-2"
    );

    // Validate msg-2 detections
    assert_eq!(
        messages[2].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(4),
                    end: Some(18),
                    text: Some("(617) 985-3519".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-2"
    );
    // Validate msg-2 usage
    assert_eq!(
        messages[2].usage,
        Some(Usage {
            prompt_tokens: 10,
            total_tokens: 21,
            completion_tokens: 11,
            ..Default::default()
        }),
        "unexpected usage for msg-2"
    );

    // Validate finish reason
    assert_eq!(
        messages[2].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_n2() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?",
            "n": 2
        }));
        then.text_stream(sse([
            Completion {
                // 0
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "Here".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 1
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 1,
                    text: "Here".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 2
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " are".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 3
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 1,
                    text: " are".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 4
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " two".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 5
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 1,
                    text: " ".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 6
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " random".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 7
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 1,
                    text: "2".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 8
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " phone".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 9
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 1,
                    text: " random".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 10
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " numbers".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 11
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 1,
                    text: " phone".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 12
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: ":\n\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 13
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 1,
                    text: " numbers".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 14
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "1. (503) 272-8192\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 15
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 1,
                    text: ":\n\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 16
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2. (617) 985-3519.".into(),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 17
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 1,
                    text: "**Phone Number 1:** (234) 567-8901\n\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                // 18
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 1,
                    text: "**Phone Number 2:** (819) 345-2198".into(),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    // choice 0 mocks
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Here".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " two".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " random".into(),
                    input_index_stream: 6,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " phone".into(),
                    input_index_stream: 8,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " numbers".into(),
                    input_index_stream: 10,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: ":\n\n".into(),
                    input_index_stream: 12,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "1. (503) 272-8192\n".into(),
                    input_index_stream: 14,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2. (617) 985-3519.".into(),
                    input_index_stream: 16,
                },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 34,
                    text: "Here are two random phone numbers:".into(),
                }],
                token_count: 0,
                processed_index: 34,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 12,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 34,
                    end: 53,
                    text: "\n\n1. (503) 278-9123".into(),
                }],
                token_count: 0,
                processed_index: 53,
                start_index: 34,
                input_start_index: 14,
                input_end_index: 14,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 53,
                    end: 72,
                    text: "\n2. (617) 854-6279.".into(),
                }],
                token_count: 0,
                processed_index: 72,
                start_index: 53,
                input_start_index: 16,
                input_end_index: 16,
            },
        ]);
    });
    // choice 1 mocks
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Here".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " ".into(),
                    input_index_stream: 5,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2".into(),
                    input_index_stream: 7,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " random".into(),
                    input_index_stream: 9,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " phone".into(),
                    input_index_stream: 11,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " numbers".into(),
                    input_index_stream: 13,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: ":\n\n".into(),
                    input_index_stream: 15,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "**Phone Number 1:** (234) 567-8901\n\n".into(),
                    input_index_stream: 17,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "**Phone Number 2:** (819) 345-2198".into(),
                    input_index_stream: 18,
                },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:".into(),
                }],
                token_count: 0,
                processed_index: 32,
                start_index: 0,
                input_start_index: 1,
                input_end_index: 15,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 32,
                    end: 68,
                    text: "\n\n**Phone Number 1:** (234) 567-8901".into(),
                }],
                token_count: 0,
                processed_index: 68,
                start_index: 32,
                input_start_index: 17,
                input_end_index: 17,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 68,
                    end: 104,
                    text: "\n\n**Phone Number 2:** (819) 345-2198".into(),
                }],
                token_count: 0,
                processed_index: 104,
                start_index: 68,
                input_start_index: 18,
                input_end_index: 18,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);
    // choice 0 mocks
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["Here are two random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n\n1. (503) 278-9123".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 5,
                "end": 19,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(503) 278-9123",
                "evidences": []
            }
        ]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n2. (617) 854-6279.".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 4,
                "end": 18,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(617) 854-6279",
                "evidences": []
            }
        ]]));
    });
    // choice 1 mocks
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["Here are 2 random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n\n**Phone Number 1:** (234) 567-8901".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 22,
                "end": 36,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(234) 567-8901",
                "evidences": []
            }
        ]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n\n**Phone Number 2:** (819) 345-2198".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 22,
                "end": 36,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(819) 345-2198",
                "evidences": []
            }
        ]]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_sentence_server])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "n": 2,
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_SENTENCE: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?",
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    let (choice0_messages, choice1_messages): (Vec<_>, Vec<_>) = messages
        .into_iter()
        .partition(|chunk| chunk.choices[0].index == 0);

    //////////////////////
    // Validate choice0 //
    //////////////////////

    // Validate length
    assert_eq!(
        choice0_messages.len(),
        3,
        "choice0: unexpected number of messages"
    );
    // Validate msg-0 choice
    assert_eq!(
        choice0_messages[0].choices,
        vec![CompletionChoice {
            index: 0,
            text: "Here are two random phone numbers:".into(),
            ..Default::default()
        }],
        "choice0: unexpected msg-0 choice"
    );
    // Validate msg-0 detections
    assert_eq!(
        choice0_messages[0].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![],
            }],
        }),
        "choice0: unexpected msg-0 detections"
    );
    // Validate msg-1 detections
    assert_eq!(
        choice0_messages[1].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(5),
                    end: Some(19),
                    text: Some("(503) 278-9123".into()),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    detection_type: "pii".into(),
                    detection: "PhoneNumber".into(),
                    score: 0.8,
                    ..Default::default()
                }]
            }],
        }),
        "choice0: unexpected msg-1 detections"
    );
    // Validate msg-0 detections
    assert_eq!(
        choice0_messages[2].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(4),
                    end: Some(18),
                    text: Some("(617) 854-6279".into()),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    detection_type: "pii".into(),
                    detection: "PhoneNumber".into(),
                    score: 0.8,
                    ..Default::default()
                }]
            }],
        }),
        "choice0: unexpected msg-2 detections"
    );

    //////////////////////
    // Validate choice1 //
    //////////////////////

    // Validate length
    assert_eq!(
        choice1_messages.len(),
        3,
        "choice1: unexpected number of messages"
    );

    // Validate choice1 choices
    assert_eq!(
        choice1_messages[0].choices,
        vec![CompletionChoice {
            index: 1,
            text: "Here are 2 random phone numbers:".into(),
            ..Default::default()
        }],
        "choice1: unexpected msg-0 choice"
    );
    assert_eq!(
        choice1_messages[1].choices,
        vec![CompletionChoice {
            index: 1,
            text: "\n\n**Phone Number 1:** (234) 567-8901".into(),
            ..Default::default()
        }],
        "choice1: unexpected msg-1 choice"
    );
    assert_eq!(
        choice1_messages[2].choices,
        vec![CompletionChoice {
            index: 1,
            text: "\n\n**Phone Number 2:** (819) 345-2198".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        }],
        "choice1: unexpected msg-2 choice"
    );

    // Validate choice1 detections
    assert_eq!(
        choice1_messages[0].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 1,
                results: vec![],
            }],
        }),
        "choice1: unexpected msg-0 detections"
    );
    assert_eq!(
        choice1_messages[1].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 1,
                results: vec![Detection {
                    start: Some(22),
                    end: Some(36),
                    text: Some("(234) 567-8901".into()),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    detection_type: "pii".into(),
                    detection: "PhoneNumber".into(),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "choice1: unexpected msg-1 detections"
    );
    assert_eq!(
        choice1_messages[2].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 1,
                results: vec![Detection {
                    start: Some(22),
                    end: Some(36),
                    text: Some("(819) 345-2198".into()),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    detection_type: "pii".into(),
                    detection: "PhoneNumber".into(),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "choice1: unexpected msg-2 detections"
    );

    Ok(())
}

#[test(tokio::test)]
async fn whole_doc_output_detectors() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?"
        }));
        then.text_stream(sse([
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "Here".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " are".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " ".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " random".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " phone".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " numbers".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: ":\n\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "1. (503) 272-8192\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2. (617) 985-3519.".into(),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let mut pii_detector_whole_doc_server = MockServer::new_http(PII_DETECTOR_WHOLE_DOC);
    pii_detector_whole_doc_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_WHOLE_DOC)
            .json(ContentAnalysisRequest {
                contents: vec![
                    "Here are 2 random phone numbers:\n\n1. (503) 272-8192\n2. (617) 985-3519."
                        .into(),
                ],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 37,
                "end": 53,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(503) 272-8192\n2",
                "evidences": []
            },
            {
                "start": 55,
                "end": 69,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(617) 985-3519",
                "evidences": []
            }
        ]]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .detector_servers([&pii_detector_whole_doc_server])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_WHOLE_DOC: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?"
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 11, "unexpected number of messages");

    // Validate finish reason
    assert_eq!(
        messages[9].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason"
    );

    // Validate whole doc detections message
    let last = &messages[10];
    assert_eq!(
        last.detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![
                    Detection {
                        start: Some(37),
                        end: Some(53),
                        text: Some("(503) 272-8192\n2".into()),
                        detection: "PhoneNumber".into(),
                        detection_type: "pii".into(),
                        detector_id: Some("pii_detector_whole_doc".into()),
                        score: 0.8,
                        ..Default::default()
                    },
                    Detection {
                        start: Some(55),
                        end: Some(69),
                        text: Some("(617) 985-3519".into()),
                        detection: "PhoneNumber".into(),
                        detection_type: "pii".into(),
                        detector_id: Some("pii_detector_whole_doc".into()),
                        score: 0.8,
                        ..Default::default()
                    }
                ],
            }],
        }),
        "unexpected whole doc detections message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_and_whole_doc_output_detectors() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?"
        }));
        then.text_stream(sse([
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "Here".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " are".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " ".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " random".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " phone".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " numbers".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: ":\n\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "1. (503) 272-8192\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2. (617) 985-3519.".into(),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Here".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " ".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " random".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " phone".into(),
                    input_index_stream: 5,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " numbers".into(),
                    input_index_stream: 6,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: ":\n\n".into(),
                    input_index_stream: 7,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "1. (503) 272-8192\n".into(),
                    input_index_stream: 8,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2. (617) 985-3519.".into(),
                    input_index_stream: 9,
                },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:".into(),
                }],
                token_count: 0,
                processed_index: 32,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 7,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 32,
                    end: 51,
                    text: "\n\n1. (503) 272-8192".into(),
                }],
                token_count: 0,
                processed_index: 51,
                start_index: 32,
                input_start_index: 8,
                input_end_index: 8,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 51,
                    end: 70,
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 9,
                input_end_index: 9,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["Here are 2 random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n\n1. (503) 272-8192".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 5,
                "end": 19,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(503) 272-8192",
                "evidences": []
            }
        ]]));
    });
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["\n2. (617) 985-3519.".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 4,
                "end": 18,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(617) 985-3519",
                "evidences": []
            }
        ]]));
    });

    let mut pii_detector_whole_doc_server = MockServer::new_http(PII_DETECTOR_WHOLE_DOC);
    pii_detector_whole_doc_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_WHOLE_DOC)
            .json(ContentAnalysisRequest {
                contents: vec![
                    "Here are 2 random phone numbers:\n\n1. (503) 272-8192\n2. (617) 985-3519."
                        .into(),
                ],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([
        [
            {
                "start": 37,
                "end": 53,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(503) 272-8192\n2",
                "evidences": []
            },
            {
                "start": 55,
                "end": 69,
                "detection": "PhoneNumber",
                "detection_type": "pii",
                "score": 0.8,
                "text": "(617) 985-3519",
                "evidences": []
            }
        ]]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([
            &pii_detector_sentence_server,
            &pii_detector_whole_doc_server,
        ])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_SENTENCE: {},
                    PII_DETECTOR_WHOLE_DOC: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?"
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 4, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![CompletionChoice {
            index: 0,
            text: "Here are 2 random phone numbers:".into(),
            ..Default::default()
        }],
        "unexpected choices for msg-0"
    );
    // Validate msg-0 detections
    assert_eq!(
        messages[0].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![],
            }],
        }),
        "unexpected detections for msg-0"
    );

    // Validate msg-1 choices
    assert_eq!(
        messages[1].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n\n1. (503) 272-8192".into(),
            ..Default::default()
        }],
        "unexpected choices for msg-1"
    );
    // Validate msg-2 detections
    assert_eq!(
        messages[1].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(5),
                    end: Some(19),
                    text: Some("(503) 272-8192".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-1"
    );

    // Validate msg-2 choices
    assert_eq!(
        messages[2].choices,
        vec![CompletionChoice {
            index: 0,
            text: "\n2. (617) 985-3519.".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        }],
        "unexpected choices for msg-2"
    );
    // Validate msg-2 detections
    assert_eq!(
        messages[2].detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![Detection {
                    start: Some(4),
                    end: Some(18),
                    text: Some("(617) 985-3519".into()),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some(PII_DETECTOR_SENTENCE.into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-2"
    );

    // Validate whole doc detections message
    let last = &messages[3];
    assert_eq!(
        last.detections,
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![
                    Detection {
                        start: Some(37),
                        end: Some(53),
                        text: Some("(503) 272-8192\n2".into()),
                        detection: "PhoneNumber".into(),
                        detection_type: "pii".into(),
                        detector_id: Some("pii_detector_whole_doc".into()),
                        score: 0.8,
                        ..Default::default()
                    },
                    Detection {
                        start: Some(55),
                        end: Some(69),
                        text: Some("(617) 985-3519".into()),
                        detection: "PhoneNumber".into(),
                        detection_type: "pii".into(),
                        detector_id: Some("pii_detector_whole_doc".into()),
                        score: 0.8,
                        ..Default::default()
                    }
                ],
            }],
        }),
        "unexpected whole doc detections message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn openai_bad_request_error() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": model_id,
                "prompt": "Hey v1",
                "prompt_logprobs": true
            })
        );
        then.bad_request().json(ErrorResponse::new_v1(
            400,
            r#"[{'type': 'value_error', 'loc': ('body',), 'msg': 'Value error, `prompt_logprobs` are not available when `stream=True`.', 'input': {'model': 'test-0B', 'messages': [{'role': 'user', 'content': 'Hey'}],'n': 1, 'seed': 1337, 'stream': True, 'prompt_logprobs': True}, 'ctx': {'error': ValueError('`prompt_logprobs` are not available when `stream=True`.')}}]"#.into(),
            "BadRequestError".into(),
            None
        ));
    });
    openai_server.mock(|when, then| {
        when.post()
            .path(COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": model_id,
                "prompt": "Hey v2",
                "prompt_logprobs": true
            })
        );
        then.bad_request().json(ErrorResponse::new_v2(
            400,
            r#"[{'type': 'value_error', 'loc': ('body',), 'msg': 'Value error, `prompt_logprobs` are not available when `stream=True`.', 'input': {'model': 'test-0B', 'messages': [{'role': 'user', 'content': 'Hey'}],'n': 1, 'seed': 1337, 'stream': True, 'prompt_logprobs': True}, 'ctx': {'error': ValueError('`prompt_logprobs` are not available when `stream=True`.')}}]"#.into(),
            "BadRequestError".into(),
            None
        ));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {},
            },
            "prompt": "Hey v1",
            "prompt_logprobs": true
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.collect::<Vec<_>>().await;

    // Validate length
    assert_eq!(messages.len(), 1, "unexpected number of messages");

    // Validate error message
    assert!(
        messages[0]
            .as_ref()
            .is_err_and(|e| e.code == http::StatusCode::BAD_REQUEST)
    );

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {},
            },
            "prompt": "Hey v2",
            "prompt_logprobs": true
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.collect::<Vec<_>>().await;

    // Validate length
    assert_eq!(messages.len(), 1, "unexpected number of messages");

    // Validate error message
    assert!(
        messages[0]
            .as_ref()
            .is_err_and(|e| e.code == http::StatusCode::BAD_REQUEST)
    );

    Ok(())
}

#[test(tokio::test)]
async fn openai_runtime_error() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Hey"
        }));
        // Return an error message over the stream
        then.text_stream(sse([ErrorResponse::new_v2(
            500,
            "unexpected error occurred".into(),
            "InternalServerError".into(),
            None,
        )]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {},
            },
            "prompt": "Hey"
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.collect::<Vec<_>>().await;

    // Validate length
    assert_eq!(messages.len(), 1, "unexpected number of messages");

    // Validate error message
    assert!(
        messages[0]
            .as_ref()
            .is_err_and(|e| e.code == StatusCode::INTERNAL_SERVER_ERROR)
    );

    Ok(())
}

#[test(tokio::test)]
async fn chunker_internal_server_error() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?"
        }));
        then.text_stream(sse([
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "Here".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " are".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " ".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " random".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " phone".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " numbers".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: ":\n\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "1. (503) 272-8192\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2. (617) 985-3519.".into(),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Here".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " ".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " random".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " phone".into(),
                    input_index_stream: 5,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " numbers".into(),
                    input_index_stream: 6,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: ":\n\n".into(),
                    input_index_stream: 7,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "1. (503) 272-8192\n".into(),
                    input_index_stream: 8,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2. (617) 985-3519.".into(),
                    input_index_stream: 9,
                },
            ]);
        then.internal_server_error();
    });

    let pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_sentence_server])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_SENTENCE: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?"
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.collect::<Vec<_>>().await;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 1, "unexpected number of messages");

    // Validate error message
    assert!(
        messages[0]
            .as_ref()
            .is_err_and(|e| e.code == StatusCode::INTERNAL_SERVER_ERROR)
    );

    Ok(())
}

#[test(tokio::test)]
async fn detector_internal_server_error() -> Result<(), anyhow::Error> {
    let model_id = "test-0B";
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post().path(COMPLETIONS_ENDPOINT).json(json!({
            "stream": true,
            "model": model_id,
            "prompt": "Can you generate 2 random phone numbers?"
        }));
        then.text_stream(sse([
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "Here".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " are".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " ".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " random".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " phone".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: " numbers".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: ":\n\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "1. (503) 272-8192\n".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Completion {
                id: "cmpl-test".into(),
                created: 1749227854,
                model: model_id.into(),
                choices: vec![CompletionChoice {
                    index: 0,
                    text: "2. (617) 985-3519.".into(),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc(CHUNKER_NAME_SENTENCE);
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE)
            .pb_stream(vec![
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Here".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " are".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " ".into(),
                    input_index_stream: 2,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2".into(),
                    input_index_stream: 3,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " random".into(),
                    input_index_stream: 4,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " phone".into(),
                    input_index_stream: 5,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " numbers".into(),
                    input_index_stream: 6,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: ":\n\n".into(),
                    input_index_stream: 7,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "1. (503) 272-8192\n".into(),
                    input_index_stream: 8,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "2. (617) 985-3519.".into(),
                    input_index_stream: 9,
                },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:".into(),
                }],
                token_count: 0,
                processed_index: 32,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 7,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 32,
                    end: 51,
                    text: "\n\n1. (503) 272-8192".into(),
                }],
                token_count: 0,
                processed_index: 51,
                start_index: 32,
                input_start_index: 8,
                input_end_index: 8,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 51,
                    end: 70,
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 9,
                input_end_index: 9,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http(PII_DETECTOR_SENTENCE);
    pii_detector_sentence_server.mock(|when, then| {
        when.post()
            .path(TEXT_CONTENTS_DETECTOR_ENDPOINT)
            .header("detector-id", PII_DETECTOR_SENTENCE)
            .json(ContentAnalysisRequest {
                contents: vec!["Here are 2 random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.internal_server_error();
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_sentence_server])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "detectors": {
                "input": {},
                "output": {
                    PII_DETECTOR_SENTENCE: {},
                },
            },
            "prompt": "Can you generate 2 random phone numbers?"
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.collect::<Vec<_>>().await;

    // Validate length
    assert_eq!(messages.len(), 1, "unexpected number of messages");

    // Validate error message
    assert!(
        messages[0]
            .as_ref()
            .is_err_and(|e| e.code == StatusCode::INTERNAL_SERVER_ERROR)
    );

    Ok(())
}
