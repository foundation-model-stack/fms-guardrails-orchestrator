pub mod common;
use common::orchestrator::*;
use fms_guardrails_orchestr8::{
    clients::{
        detector::{ContentAnalysisRequest, ContentAnalysisResponse},
        openai::{
            ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionDelta, ChatDetections,
            Content, Message, OutputDetectionResult, Role,
        },
    },
    models::DetectorParams,
    pb::{
        caikit::runtime::chunkers::BidiStreamingChunkerTokenizationTaskRequest,
        caikit_data_model::nlp::{ChunkerTokenizationStreamResult, Token},
    },
};
use futures::TryStreamExt;
use mocktail::prelude::*;
use serde::Serialize;
use serde_json::json;
use test_log::test;
use tracing::debug;

#[test(tokio::test)]
async fn no_detectors_n1() -> Result<(), anyhow::Error> {
    let mut chat_completions_server = MockServer::new("chat_completions");
    chat_completions_server.mock(|when, then| {
        when
        .post()
        .path("/v1/chat/completions")
        .json(json!({
            "stream": true,
            "model": "test-0B",
            "messages": [
                Message { role: Role::Assistant, content: Some(Content::Text("You are a helpful assistant.".into())), ..Default::default() },
                Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default() },
            ]
        }));
        then.text_stream(sse([
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        role: Some(Role::Assistant),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("Hey".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("!".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path("tests/test_config.yaml")
        .chat_completions_server(&chat_completions_server)
        .build()
        .await?;

    let response = test_server
        .post("/api/v2/chat/completions-detection")
        .json(&json!({
            "stream": true,
            "model": "test-0B",
            "messages": [
                Message { role: Role::Assistant, content: Some(Content::Text("You are a helpful assistant.".into())), ..Default::default() },
                Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default() },
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].choices[0].delta.role, Some(Role::Assistant));
    assert_eq!(messages[1].choices[0].delta.content, Some("Hey".into()));
    assert_eq!(messages[2].choices[0].delta.content, Some("!".into()));
    assert_eq!(messages[3].choices[0].finish_reason, Some("stop".into()));

    Ok(())
}

#[test(tokio::test)]
async fn no_detectors_n2() -> Result<(), anyhow::Error> {
    let mut chat_completions_server = MockServer::new("chat_completions");
    chat_completions_server.mock(|when, then| {
        when.post()
            .path("/v1/chat/completions")
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default() },
                ],
                "n": 2,
            })
        );
        then.text_stream(sse([
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        role: Some(Role::Assistant),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        role: Some(Role::Assistant),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("Hey".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some("Hey".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("!".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path("tests/test_config.yaml")
        .chat_completions_server(&chat_completions_server)
        .build()
        .await?;

    let response = test_server
        .post("/api/v2/chat/completions-detection")
        .json(&json!({
            "n": 2,
            "stream": true,
            "model": "test-0B",
            "messages": [
                Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default() },
            ]
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 7);

    // Validate role messages for both choices
    assert_eq!(messages[0].choices[0].index, 0);
    assert_eq!(messages[0].choices[0].delta.role, Some(Role::Assistant));
    assert_eq!(messages[1].choices[0].index, 1);
    assert_eq!(messages[1].choices[0].delta.role, Some(Role::Assistant));

    // Validate content messages for both choices
    assert_eq!(messages[2].choices[0].index, 0);
    assert_eq!(messages[2].choices[0].delta.content, Some("Hey".into()));
    assert_eq!(messages[3].choices[0].index, 1);
    assert_eq!(messages[3].choices[0].delta.content, Some("Hey".into()));
    assert_eq!(messages[4].choices[0].index, 0);
    assert_eq!(messages[4].choices[0].delta.content, Some("!".into()));

    // Validate stop messages for both choices
    assert_eq!(messages[5].choices[0].index, 0);
    assert_eq!(messages[5].choices[0].finish_reason, Some("stop".into()));
    assert_eq!(messages[6].choices[0].index, 1);
    assert_eq!(messages[6].choices[0].finish_reason, Some("stop".into()));

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_n1() -> Result<(), anyhow::Error> {
    let mut chat_completions_server = MockServer::new("chat_completions");
    chat_completions_server.mock(|when, then| {
        when.post()
            .path("/v1/chat/completions")
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default() },
                ]
            })
        );
        then.text_stream(sse([
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        role: Some(Role::Assistant),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("Here".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(" are".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(" ".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("2".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(" random".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(" phone".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(" numbers".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(":\n\n".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("1. (503) 272-8192\n".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("2. (617) 985-3519.".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new("sentence_chunker").grpc();
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path("/caikit.runtime.Chunkers.ChunkersService/BidiStreamingChunkerTokenizationTaskPredict")
            .header("mm-model-id", "sentence_chunker")
            .pb_stream(vec![
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "Here".into(),
                input_index_stream: 1,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " are".into(),
                input_index_stream: 2,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " ".into(),
                input_index_stream: 3,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "2".into(),
                input_index_stream: 4,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " random".into(),
                input_index_stream: 5,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " phone".into(),
                input_index_stream: 6,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " numbers".into(),
                input_index_stream: 7,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: ":\n\n".into(),
                input_index_stream: 8,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "1. (503) 272-8192\n".into(),
                input_index_stream: 9,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "2. (617) 985-3519.".into(),
                input_index_stream: 10,
            },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:"
                        .into(),
                }],
                token_count: 0,
                processed_index: 32,
                start_index: 0,
                input_start_index: 1,
                input_end_index: 8,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 32,
                    end: 51,
                    text: "\n\n1. (503) 272-8192"
                        .into(),
                }],
                token_count: 0,
                processed_index: 51,
                start_index: 32,
                input_start_index: 9,
                input_end_index: 9,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 51,
                    end: 70,
                    text: "\n2. (617) 985-3519."
                        .into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 10,
                input_end_index: 10,
            },
        ]);
    });

    let mut pii_detector_server = MockServer::new("pii_detector_sentence");
    pii_detector_server.mock(|when, then| {
        when.post()
            .path("/api/v1/text/contents")
            .header("detector-id", "pii_detector_sentence")
            .json(ContentAnalysisRequest {
                contents: vec!["Here are 2 random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_server.mock(|when, then| {
        when.post()
            .path("/api/v1/text/contents")
            .header("detector-id", "pii_detector_sentence")
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
    pii_detector_server.mock(|when, then| {
        when.post()
            .path("/api/v1/text/contents")
            .header("detector-id", "pii_detector_sentence")
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
        .config_path("tests/test_config.yaml")
        .chat_completions_server(&chat_completions_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_server])
        .build()
        .await?;

    let response = test_server
        .post("/api/v2/chat/completions-detection")
        .json(&json!({
            "stream": true,
            "model": "test-0B",
            "detectors": {
                "input": {},
                "output": {
                    "pii_detector_sentence": {},
                },
            },
            "messages": [
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default() },
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 3, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: None,
                content: Some("Here are 2 random phone numbers:".into(),),
                refusal: None,
                tool_calls: vec![],
            },
            ..Default::default()
        }],
        "unexpected choices for msg-0"
    );
    // Validate msg-0 detections
    assert_eq!(
        messages[0].detections,
        Some(ChatDetections {
            input: vec![],
            output: vec![OutputDetectionResult {
                choice_index: 0,
                results: vec![],
            }],
        }),
        "unexpected detections for msg-0"
    );

    // Validate msg-1 choices
    assert_eq!(
        messages[1].choices,
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: None,
                content: Some("\n\n1. (503) 272-8192".into(),),
                refusal: None,
                tool_calls: vec![],
            },
            ..Default::default()
        }],
        "unexpected choices for msg-1"
    );
    // Validate msg-2 detections
    assert_eq!(
        messages[1].detections,
        Some(ChatDetections {
            input: vec![],
            output: vec![OutputDetectionResult {
                choice_index: 0,
                results: vec![ContentAnalysisResponse {
                    start: 5,
                    end: 19,
                    text: "(503) 272-8192".into(),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some("pii_detector_sentence".into()),
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
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: None,
                content: Some("\n2. (617) 985-3519.".into(),),
                refusal: None,
                tool_calls: vec![],
            },
            ..Default::default()
        }],
        "unexpected choices for msg-2"
    );
    // Validate msg-2 detections
    assert_eq!(
        messages[2].detections,
        Some(ChatDetections {
            input: vec![],
            output: vec![OutputDetectionResult {
                choice_index: 0,
                results: vec![ContentAnalysisResponse {
                    start: 4,
                    end: 18,
                    text: "(617) 985-3519".into(),
                    detection: "PhoneNumber".into(),
                    detection_type: "pii".into(),
                    detector_id: Some("pii_detector_sentence".into()),
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
async fn output_detectors_n2() -> Result<(), anyhow::Error> {
    let mut chat_completions_server = MockServer::new("chat_completions");
    chat_completions_server.mock(|when, then| {
        when.post()
            .path("/v1/chat/completions")
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default() },
                ],
                "n": 2,
            })
        );
        then.text_stream(sse([
            ChatCompletionChunk { // 0
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        role: Some(Role::Assistant),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 1
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        role: Some(Role::Assistant),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 2
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("Here".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 3
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some("Here".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 4
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(" are".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 5
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some(" are".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 6
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(" two".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 7
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some(" ".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 8
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(" random".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 9
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some("2".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 10
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(" phone".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 11
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some(" random".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 12
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(" numbers".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 13
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some(" phone".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 14
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some(":\n\n".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 15
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some(" numbers".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 16
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("1. (503) 278-9123\n".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 17
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some(":\n\n".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 18
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatCompletionDelta {
                        content: Some("2. (617) 854-6279.".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 19
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some("**Phone Number 1:** (234) 567-8901\n\n".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 20
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    delta: ChatCompletionDelta {
                        content: Some("**Phone Number 2:** (819) 345-2198".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 21
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ChatCompletionChunk { // 22
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 1,
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new("sentence_chunker").grpc();
    // choice 0 mocks
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path("/caikit.runtime.Chunkers.ChunkersService/BidiStreamingChunkerTokenizationTaskPredict")
            .header("mm-model-id", "sentence_chunker")
            .pb_stream(vec![
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "Here".into(),
                input_index_stream: 2,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " are".into(),
                input_index_stream: 4,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " two".into(),
                input_index_stream: 6,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " random".into(),
                input_index_stream: 8,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " phone".into(),
                input_index_stream: 10,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " numbers".into(),
                input_index_stream: 12,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: ":\n\n".into(),
                input_index_stream: 14,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "1. (503) 278-9123\n".into(),
                input_index_stream: 16,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "2. (617) 854-6279.".into(),
                input_index_stream: 18,
            },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 34,
                    text: "Here are two random phone numbers:"
                        .into(),
                }],
                token_count: 0,
                processed_index: 34,
                start_index: 0,
                input_start_index: 2,
                input_end_index: 14,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 34,
                    end: 53,
                    text: "\n\n1. (503) 278-9123"
                        .into(),
                }],
                token_count: 0,
                processed_index: 53,
                start_index: 34,
                input_start_index: 16,
                input_end_index: 16,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 53,
                    end: 72,
                    text: "\n2. (617) 854-6279."
                        .into(),
                }],
                token_count: 0,
                processed_index: 72,
                start_index: 53,
                input_start_index: 18,
                input_end_index: 18,
            },
        ]);
    });
    // choice 1 mocks
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path("/caikit.runtime.Chunkers.ChunkersService/BidiStreamingChunkerTokenizationTaskPredict")
            .header("mm-model-id", "sentence_chunker")
            .pb_stream(vec![
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "Here".into(),
                input_index_stream: 3,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " are".into(),
                input_index_stream: 5,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " ".into(),
                input_index_stream: 7,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "2".into(),
                input_index_stream: 9,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " random".into(),
                input_index_stream: 11,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " phone".into(),
                input_index_stream: 13,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: " numbers".into(),
                input_index_stream: 15,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: ":\n\n".into(),
                input_index_stream: 17,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "**Phone Number 1:** (234) 567-8901\n\n".into(),
                input_index_stream: 19,
            },
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: "**Phone Number 2:** (819) 345-2198".into(),
                input_index_stream: 20,
            },
            ]);
        then.pb_stream(vec![
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 32,
                    text: "Here are 2 random phone numbers:"
                        .into(),
                }],
                token_count: 0,
                processed_index: 32,
                start_index: 0,
                input_start_index: 3,
                input_end_index: 17,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 32,
                    end: 68,
                    text: "\n\n**Phone Number 1:** (234) 567-8901"
                        .into(),
                }],
                token_count: 0,
                processed_index: 68,
                start_index: 32,
                input_start_index: 19,
                input_end_index: 19,
            },
            ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 68,
                    end: 104,
                    text: "\n\n**Phone Number 2:** (819) 345-2198"
                        .into(),
                }],
                token_count: 0,
                processed_index: 104,
                start_index: 68,
                input_start_index: 20,
                input_end_index: 20,
            },
        ]);
    });

    let mut pii_detector_server = MockServer::new("pii_detector_sentence");
    // choice 0 mocks
    pii_detector_server.mock(|when, then| {
        when.post()
            .path("/api/v1/text/contents")
            .header("detector-id", "pii_detector_sentence")
            .json(ContentAnalysisRequest {
                contents: vec!["Here are two random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_server.mock(|when, then| {
        when.post()
            .path("/api/v1/text/contents")
            .header("detector-id", "pii_detector_sentence")
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
    pii_detector_server.mock(|when, then| {
        when.post()
            .path("/api/v1/text/contents")
            .header("detector-id", "pii_detector_sentence")
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
    pii_detector_server.mock(|when, then| {
        when.post()
            .path("/api/v1/text/contents")
            .header("detector-id", "pii_detector_sentence")
            .json(ContentAnalysisRequest {
                contents: vec!["Here are 2 random phone numbers:".into()],
                detector_params: DetectorParams::default(),
            });
        then.json(json!([[]]));
    });
    pii_detector_server.mock(|when, then| {
        when.post()
            .path("/api/v1/text/contents")
            .header("detector-id", "pii_detector_sentence")
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
    pii_detector_server.mock(|when, then| {
        when.post()
            .path("/api/v1/text/contents")
            .header("detector-id", "pii_detector_sentence")
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
        .config_path("tests/test_config.yaml")
        .chat_completions_server(&chat_completions_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_server])
        .build()
        .await?;

    let response = test_server
        .post("/api/v2/chat/completions-detection")
        .json(&json!({
            "n": 2,
            "stream": true,
            "model": "test-0B",
            "detectors": {
                "input": {},
                "output": {
                    "pii_detector_sentence": {},
                },
            },
            "messages": [
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default() },
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    let (choice0_messages, choice1_messages): (Vec<_>, Vec<_>) = messages
        .into_iter()
        .partition(|chunk| chunk.choices[0].index == 0);

    // Validate choice0 messages:
    // Validate length
    assert_eq!(
        choice0_messages.len(),
        4,
        "choice0: unexpected number of messages"
    );
    // Validate msg-0 choice
    assert_eq!(
        choice0_messages[0].choices,
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant,),
                content: Some("Here are two random phone numbers:".into()),
                refusal: None,
                tool_calls: vec![],
            },
            ..Default::default()
        }],
        "choice0: unexpected msg-0 choice"
    );
    // Validate msg-0 detections
    assert_eq!(
        choice0_messages[0].detections,
        Some(ChatDetections {
            input: vec![],
            output: vec![OutputDetectionResult {
                choice_index: 0,
                results: vec![],
            }],
        }),
        "choice0: unexpected msg-0 detections"
    );
    // Validate stop message
    assert!(
        choice0_messages
            .last()
            .is_some_and(|msg| msg.choices[0].finish_reason.is_some()),
        "choice0: missing stop message"
    );

    // Validate choice1 messages:
    // Validate length
    assert_eq!(
        choice1_messages.len(),
        4,
        "choice1: unexpected number of messages"
    );
    // Validate msg-0 choice
    assert_eq!(
        choice1_messages[0].choices,
        vec![ChatCompletionChunkChoice {
            index: 1,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant,),
                content: Some("Here are 2 random phone numbers:".into()),
                refusal: None,
                tool_calls: vec![],
            },
            ..Default::default()
        }],
        "choice1: unexpected msg-0 choice"
    );
    // Validate msg-0 detections
    assert_eq!(
        choice1_messages[0].detections,
        Some(ChatDetections {
            input: vec![],
            output: vec![OutputDetectionResult {
                choice_index: 1,
                results: vec![],
            }],
        }),
        "choice1: unexpected msg-0 detections"
    );
    // Validate stop message
    assert!(
        choice1_messages
            .last()
            .is_some_and(|msg| msg.choices[0].finish_reason.is_some()),
        "choice1: missing stop message"
    );

    Ok(())
}

/// Converts an iterator of serializable messages into an iterator of SSE data messages.
fn sse(messages: impl IntoIterator<Item = impl Serialize>) -> impl IntoIterator<Item = String> {
    messages.into_iter().map(|msg| {
        let msg = serde_json::to_string(&msg).unwrap();
        format!("data: {msg}\n\n")
    })
}
