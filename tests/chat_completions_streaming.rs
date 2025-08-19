pub mod common;
use common::orchestrator::*;
use fms_guardrails_orchestr8::{
    clients::{
        detector::ContentAnalysisRequest,
        openai::{
            ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionDelta,
            ChatCompletionLogprob, ChatCompletionLogprobs, CompletionDetections,
            CompletionInputDetections, CompletionOutputDetections, Content, Message, OpenAiError,
            OpenAiErrorMessage, Role, TokenizeResponse, Usage,
        },
    },
    models::DetectorParams,
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
    chunker::{CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_STREAMING_ENDPOINT, CHUNKER_UNARY_ENDPOINT},
    detectors::{PII_DETECTOR_SENTENCE, PII_DETECTOR_WHOLE_DOC, TEXT_CONTENTS_DETECTOR_ENDPOINT},
    openai::{CHAT_COMPLETIONS_ENDPOINT, TOKENIZE_ENDPOINT},
    sse,
};

#[test(tokio::test)]
async fn no_detectors() -> Result<(), anyhow::Error> {
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when
        .post()
        .path(CHAT_COMPLETIONS_ENDPOINT)
        .json(json!({
            "stream": true,
            "model": "test-0B",
            "messages": [
                Message { role: Role::Assistant, content: Some(Content::Text("You are a helpful assistant.".into())), ..Default::default()},
                Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default()},
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
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": "test-0B",
            "messages": [
                Message { role: Role::Assistant, content: Some(Content::Text("You are a helpful assistant.".into())), ..Default::default()},
                Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default()},
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    assert_eq!(messages.len(), 4);
    assert_eq!(
        messages[0].choices[0].delta.role,
        Some(Role::Assistant),
        "missing role message"
    );
    assert_eq!(messages[1].choices[0].delta.content, Some("Hey".into()));
    assert_eq!(messages[2].choices[0].delta.content, Some("!".into()));
    assert_eq!(
        messages[3].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn no_detectors_n2() -> Result<(), anyhow::Error> {
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default()},
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
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "n": 2,
            "stream": true,
            "model": "test-0B",
            "messages": [
                Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default()},
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
    assert_eq!(
        messages[0].choices[0].delta.role,
        Some(Role::Assistant),
        "choice0: missing role message"
    );
    assert_eq!(messages[1].choices[0].index, 1);
    assert_eq!(
        messages[1].choices[0].delta.role,
        Some(Role::Assistant),
        "choice1: missing role message"
    );

    // Validate content messages for both choices
    assert_eq!(messages[2].choices[0].index, 0);
    assert_eq!(messages[2].choices[0].delta.content, Some("Hey".into()));
    assert_eq!(messages[3].choices[0].index, 1);
    assert_eq!(messages[3].choices[0].delta.content, Some("Hey".into()));
    assert_eq!(messages[4].choices[0].index, 0);
    assert_eq!(messages[4].choices[0].delta.content, Some("!".into()));

    // Validate stop messages for both choices
    assert_eq!(messages[5].choices[0].index, 0);
    assert_eq!(
        messages[5].choices[0].finish_reason,
        Some("stop".into()),
        "choice0: missing finish reason message"
    );
    assert_eq!(messages[6].choices[0].index, 1);
    assert_eq!(
        messages[6].choices[0].finish_reason,
        Some("stop".into()),
        "choice1: missing finish reason message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn input_detectors() -> Result<(), anyhow::Error> {
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(TOKENIZE_ENDPOINT)
            .json(json!({
            "model": "test-0B",
            "prompt": "Here is my social security number: 123-45-6789. Can you generate another one like it?",
        }));
        then.json(&TokenizeResponse {
            count: 24,
            ..Default::default()
        });
    });

    let mut sentence_chunker_server = MockServer::new_grpc("sentence_chunker");
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_UNARY_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, "sentence_chunker")
            .pb(ChunkerTokenizationTaskRequest { text: "Here is my social security number: 123-45-6789. Can you generate another one like it?".into() });
        then.pb(TokenizationResults {
            results: vec![
                Token { start: 0, end: 47, text: "Here is my social security number: 123-45-6789.".into() },
                Token { start: 48, end: 85, text: "Can you generate another one like it?".into() },
            ],
            token_count: 0
        });
    });

    let mut pii_detector_sentence_server = MockServer::new_http("pii_detector_sentence");
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
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": "test-0B",
            "detectors": {
                "input": {
                    "pii_detector_sentence": {}
                },
                "output": {}
            },
            "messages": [
                Message { role: Role::User, content: Some(Content::Text("Here is my social security number: 123-45-6789. Can you generate another one like it?".into())), ..Default::default()},
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
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
                    detector_id: Some("pii_detector_sentence".into()),
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
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
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

    let mut sentence_chunker_server = MockServer::new_grpc("sentence_chunker");
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, "sentence_chunker")
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
                    text: "Here are 2 random phone numbers:".into(),
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
                    text: "\n\n1. (503) 272-8192".into(),
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
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 10,
                input_end_index: 10,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http("pii_detector_sentence");
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
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
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
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 4, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
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
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
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
                role: Some(Role::Assistant),
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
                    detector_id: Some("pii_detector_sentence".into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-2"
    );

    // Validate finish reason message
    assert_eq!(
        messages[3].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_with_logprobs() -> Result<(), anyhow::Error> {
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
                ],
                "logprobs": true,
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
                    logprobs: Some(ChatCompletionLogprobs {
                        content: vec![ChatCompletionLogprob {
                                token: "Here".into(), 
                                logprob: -0.021,
                                bytes: None,
                                top_logprobs: None,
                            }],
                        ..Default::default()
                    }),
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
                    logprobs: Some(ChatCompletionLogprobs {
                        content: vec![ChatCompletionLogprob {
                                token: " are".into(), 
                                logprob: -0.011,
                                bytes: None,
                                top_logprobs: None,
                            }],
                        ..Default::default()
                    }),
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
                    logprobs: Some(ChatCompletionLogprobs {
                        content: vec![ChatCompletionLogprob {
                                token: " ".into(), 
                                logprob: -0.001,
                                bytes: None,
                                top_logprobs: None,
                            }],
                        ..Default::default()
                    }),
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
                    logprobs: Some(ChatCompletionLogprobs {
                        content: vec![ChatCompletionLogprob {
                                token: "2".into(), 
                                logprob: -0.003,
                                bytes: None,
                                top_logprobs: None,
                            }],
                        ..Default::default()
                    }),
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
                    logprobs: Some(ChatCompletionLogprobs {
                        content: vec![ChatCompletionLogprob {
                                token: " random".into(), 
                                logprob: -0.044,
                                bytes: None,
                                top_logprobs: None,
                            }],
                        ..Default::default()
                    }),
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
                    logprobs: Some(ChatCompletionLogprobs {
                        content: vec![ChatCompletionLogprob {
                                token: " phone".into(), 
                                logprob: -0.004,
                                bytes: None,
                                top_logprobs: None,
                            }],
                        ..Default::default()
                    }),
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
                    logprobs: Some(ChatCompletionLogprobs {
                        content: vec![ChatCompletionLogprob {
                                token: " numbers".into(), 
                                logprob: -0.005,
                                bytes: None,
                                top_logprobs: None,
                            }],
                        ..Default::default()
                    }),
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
                    logprobs: Some(ChatCompletionLogprobs {
                        content: vec![ChatCompletionLogprob {
                                token: ":\n\n".into(),
                                logprob: -0.001,
                                bytes: None,
                                top_logprobs: None,
                            }],
                        ..Default::default()
                    }),
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
                    logprobs: Some(ChatCompletionLogprobs {
                        content: vec![ChatCompletionLogprob {
                                token: "1. (503) 272-8192\n".into(), 
                                logprob: -0.066,
                                bytes: None,
                                top_logprobs: None,
                            }],
                        ..Default::default()
                    }),
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
                    logprobs: Some(ChatCompletionLogprobs {
                        content: vec![ChatCompletionLogprob {
                                token: "2. (617) 985-3519.".into(), 
                                logprob: -0.055,
                                bytes: None,
                                top_logprobs: None,
                            }],
                        ..Default::default()
                    }),
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

    let mut sentence_chunker_server = MockServer::new_grpc("sentence_chunker");
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, "sentence_chunker")
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
                    text: "Here are 2 random phone numbers:".into(),
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
                    text: "\n\n1. (503) 272-8192".into(),
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
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 10,
                input_end_index: 10,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http("pii_detector_sentence");
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
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "logprobs": true,
            "model": "test-0B",
            "detectors": {
                "input": {},
                "output": {
                    "pii_detector_sentence": {},
                },
            },
            "messages": [
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 4, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
                content: Some("Here are 2 random phone numbers:".into(),),
                refusal: None,
                tool_calls: vec![],
            },
            logprobs: Some(ChatCompletionLogprobs {
                content: vec![
                    ChatCompletionLogprob {
                        token: "Here".into(),
                        logprob: -0.021,
                        bytes: None,
                        top_logprobs: None,
                    },
                    ChatCompletionLogprob {
                        token: " are".into(),
                        logprob: -0.011,
                        bytes: None,
                        top_logprobs: None,
                    },
                    ChatCompletionLogprob {
                        token: " ".into(),
                        logprob: -0.001,
                        bytes: None,
                        top_logprobs: None,
                    },
                    ChatCompletionLogprob {
                        token: "2".into(),
                        logprob: -0.003,
                        bytes: None,
                        top_logprobs: None,
                    },
                    ChatCompletionLogprob {
                        token: " random".into(),
                        logprob: -0.044,
                        bytes: None,
                        top_logprobs: None,
                    },
                    ChatCompletionLogprob {
                        token: " phone".into(),
                        logprob: -0.004,
                        bytes: None,
                        top_logprobs: None,
                    },
                    ChatCompletionLogprob {
                        token: " numbers".into(),
                        logprob: -0.005,
                        bytes: None,
                        top_logprobs: None,
                    },
                    ChatCompletionLogprob {
                        token: ":\n\n".into(),
                        logprob: -0.001,
                        bytes: None,
                        top_logprobs: None,
                    },
                ],
                refusal: vec![],
            },),
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
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
                content: Some("\n\n1. (503) 272-8192".into(),),
                refusal: None,
                tool_calls: vec![],
            },
            logprobs: Some(ChatCompletionLogprobs {
                content: vec![ChatCompletionLogprob {
                    token: "1. (503) 272-8192\n".into(),
                    logprob: -0.066,
                    bytes: None,
                    top_logprobs: None,
                },],
                refusal: vec![],
            },),
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
                role: Some(Role::Assistant),
                content: Some("\n2. (617) 985-3519.".into(),),
                refusal: None,
                tool_calls: vec![],
            },
            logprobs: Some(ChatCompletionLogprobs {
                content: vec![ChatCompletionLogprob {
                    token: "2. (617) 985-3519.".into(),
                    logprob: -0.055,
                    bytes: None,
                    top_logprobs: None,
                },],
                refusal: vec![],
            },),
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
                    detector_id: Some("pii_detector_sentence".into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-2"
    );

    // Validate finish reason message
    assert_eq!(
        messages[3].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_with_usage() -> Result<(), anyhow::Error> {
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
                ],
                "stream_options": {
                    "include_usage": true
                }
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
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 49,
                    completion_tokens: 30,
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc("sentence_chunker");
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, "sentence_chunker")
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
                    text: "Here are 2 random phone numbers:".into(),
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
                    text: "\n\n1. (503) 272-8192".into(),
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
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 10,
                input_end_index: 10,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http("pii_detector_sentence");
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
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
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
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
            ],
            "stream_options": {
                "include_usage": true
            }
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 5, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
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
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
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
                role: Some(Role::Assistant),
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
                    detector_id: Some("pii_detector_sentence".into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-2"
    );

    // Validate finish reason message
    assert_eq!(
        messages[3].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason message"
    );

    // Validate final usage message
    assert_eq!(
        messages[4].usage,
        Some(Usage {
            prompt_tokens: 19,
            total_tokens: 49,
            completion_tokens: 30,
            ..Default::default()
        }),
        "unexpected usage for final usage message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_with_continuous_usage_stats() -> Result<(), anyhow::Error> {
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
                ],
                "stream_options": {
                    "include_usage": true,
                    "continuous_usage_stats": true
                }
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 19,
                    completion_tokens: 0,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 20,
                    completion_tokens: 1,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 21,
                    completion_tokens: 2,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 22,
                    completion_tokens: 3,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 23,
                    completion_tokens: 4,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 24,
                    completion_tokens: 5,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 25,
                    completion_tokens: 6,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 26,
                    completion_tokens: 7,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 28,
                    completion_tokens: 8,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 38,
                    completion_tokens: 19,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 49,
                    completion_tokens: 30,
                    ..Default::default()
                }),
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
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 49,
                    completion_tokens: 30,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ChatCompletionChunk {
                id: "chatcmpl-test".into(),
                object: "chat.completion.chunk".into(),
                created: 1749227854,
                model: "test-0B".into(),
                usage: Some(Usage {
                    prompt_tokens: 19,
                    total_tokens: 49,
                    completion_tokens: 30,
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]));
    });

    let mut sentence_chunker_server = MockServer::new_grpc("sentence_chunker");
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, "sentence_chunker")
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
                    text: "Here are 2 random phone numbers:".into(),
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
                    text: "\n\n1. (503) 272-8192".into(),
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
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 10,
                input_end_index: 10,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http("pii_detector_sentence");
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
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
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
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
            ],
            "stream_options": {
                "include_usage": true,
                "continuous_usage_stats": true
            }
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 5, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
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
            prompt_tokens: 19,
            total_tokens: 28,
            completion_tokens: 8,
            ..Default::default()
        }),
        "unexpected usage for msg-0"
    );

    // Validate msg-1 choices
    assert_eq!(
        messages[1].choices,
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
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
                    detector_id: Some("pii_detector_sentence".into()),
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
            prompt_tokens: 19,
            total_tokens: 38,
            completion_tokens: 19,
            ..Default::default()
        }),
        "unexpected usage for msg-1"
    );

    // Validate msg-2 choices
    assert_eq!(
        messages[2].choices,
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
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
                    detector_id: Some("pii_detector_sentence".into()),
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
            prompt_tokens: 19,
            total_tokens: 49,
            completion_tokens: 30,
            ..Default::default()
        }),
        "unexpected usage for msg-2"
    );

    // Validate finish reason message
    assert_eq!(
        messages[3].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason message"
    );
    // Validate finish reason message usage
    assert_eq!(
        messages[3].usage,
        Some(Usage {
            prompt_tokens: 19,
            total_tokens: 49,
            completion_tokens: 30,
            ..Default::default()
        }),
        "unexpected usage for finish reason message"
    );

    // Validate final usage message
    assert_eq!(
        messages[4].usage,
        Some(Usage {
            prompt_tokens: 19,
            total_tokens: 49,
            completion_tokens: 30,
            ..Default::default()
        }),
        "unexpected usage for final usage message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn output_detectors_n2() -> Result<(), anyhow::Error> {
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
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

    let mut sentence_chunker_server = MockServer::new_grpc("sentence_chunker");
    // choice 0 mocks
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, "sentence_chunker")
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
                    text: "Here are two random phone numbers:".into(),
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
                    text: "\n\n1. (503) 278-9123".into(),
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
                    text: "\n2. (617) 854-6279.".into(),
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
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, "sentence_chunker")
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
                    text: "Here are 2 random phone numbers:".into(),
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
                    text: "\n\n**Phone Number 1:** (234) 567-8901".into(),
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
                    text: "\n\n**Phone Number 2:** (819) 345-2198".into(),
                }],
                token_count: 0,
                processed_index: 104,
                start_index: 68,
                input_start_index: 20,
                input_end_index: 20,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http("pii_detector_sentence");
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
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
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
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
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
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 0,
                results: vec![],
            }],
        }),
        "choice0: unexpected msg-0 detections"
    );
    // Validate finish reason message
    assert!(
        choice0_messages
            .last()
            .is_some_and(|msg| msg.choices[0].finish_reason.is_some()),
        "choice0: missing finish reason message"
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
        Some(CompletionDetections {
            input: vec![],
            output: vec![CompletionOutputDetections {
                choice_index: 1,
                results: vec![],
            }],
        }),
        "choice1: unexpected msg-0 detections"
    );
    // Validate finish reason message
    assert!(
        choice1_messages
            .last()
            .is_some_and(|msg| msg.choices[0].finish_reason.is_some()),
        "choice1: missing finish reason message"
    );

    Ok(())
}

#[test(tokio::test)]
async fn whole_doc_output_detectors() -> Result<(), anyhow::Error> {
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
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

    let mut pii_detector_whole_doc_server = MockServer::new_http("pii_detector_whole_doc");
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
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": "test-0B",
            "detectors": {
                "input": {},
                "output": {
                    "pii_detector_whole_doc": {},
                },
            },
            "messages": [
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 13, "unexpected number of messages");

    // Validate finish reason message
    assert_eq!(
        messages[11].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason message"
    );

    // Validate whole doc detections message
    let last = &messages[12];
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
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
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

    let mut sentence_chunker_server = MockServer::new_grpc("sentence_chunker");
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, "sentence_chunker")
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
                    text: "Here are 2 random phone numbers:".into(),
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
                    text: "\n\n1. (503) 272-8192".into(),
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
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 10,
                input_end_index: 10,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http("pii_detector_sentence");
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

    let mut pii_detector_whole_doc_server = MockServer::new_http("pii_detector_whole_doc");
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
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": "test-0B",
            "detectors": {
                "input": {},
                "output": {
                    "pii_detector_sentence": {},
                    "pii_detector_whole_doc": {},
                },
            },
            "messages": [
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("{messages:#?}");

    // Validate length
    assert_eq!(messages.len(), 5, "unexpected number of messages");

    // Validate msg-0 choices
    assert_eq!(
        messages[0].choices,
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
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
        vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatCompletionDelta {
                role: Some(Role::Assistant),
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
                role: Some(Role::Assistant),
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
                    detector_id: Some("pii_detector_sentence".into()),
                    score: 0.8,
                    ..Default::default()
                }],
            }],
        }),
        "unexpected detections for msg-2"
    );

    // Validate finish reason message
    assert_eq!(
        messages[3].choices[0].finish_reason,
        Some("stop".into()),
        "missing finish reason message"
    );

    // Validate whole doc detections message
    let last = &messages[4];
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
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default()},
                ],
                "prompt_logprobs": true
            })
        );
        then.bad_request().json(OpenAiError {
            object: Some("error".into()),
            message: r#"[{'type': 'value_error', 'loc': ('body',), 'msg': 'Value error, `prompt_logprobs` are not available when `stream=True`.', 'input': {'model': 'test-0B', 'messages': [{'role': 'user', 'content': 'Hey'}],'n': 1, 'seed': 1337, 'stream': True, 'prompt_logprobs': True}, 'ctx': {'error': ValueError('`prompt_logprobs` are not available when `stream=True`.')}}]"#.into(),
            r#type: Some("BadRequestError".into()),
            param: None,
            code: 400,
        });
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": "test-0B",
            "detectors": {
                "input": {},
                "output": {},
            },
            "messages": [
                Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default()},
            ],
            "prompt_logprobs": true
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
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
async fn openai_stream_error() -> Result<(), anyhow::Error> {
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default()},
                ]
            })
        );
        // Return an error message over the stream
        then.text_stream(sse([
            OpenAiErrorMessage {
                error: OpenAiError {
                    object: Some("error".into()),
                    message: "".into(),
                    r#type: Some("InternalServerError".into()),
                    param: None,
                    code: 500
                }
            }
        ]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": "test-0B",
            "detectors": {
                "input": {},
                "output": {},
            },
            "messages": [
                Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default()},
            ]
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
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
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
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

    let mut sentence_chunker_server = MockServer::new_grpc("sentence_chunker");
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, "sentence_chunker")
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
        then.internal_server_error();
    });

    let pii_detector_sentence_server = MockServer::new_http("pii_detector_sentence");

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .openai_server(&openai_server)
        .chunker_servers([&sentence_chunker_server])
        .detector_servers([&pii_detector_sentence_server])
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
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
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
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
async fn detector_internal_server_error() -> Result<(), anyhow::Error> {
    let mut openai_server = MockServer::new_http("openai");
    openai_server.mock(|when, then| {
        when.post()
            .path(CHAT_COMPLETIONS_ENDPOINT)
            .json(json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
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

    let mut sentence_chunker_server = MockServer::new_grpc("sentence_chunker");
    sentence_chunker_server.mock(|when, then| {
        when.post()
            .path(CHUNKER_STREAMING_ENDPOINT)
            .header(CHUNKER_MODEL_ID_HEADER_NAME, "sentence_chunker")
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
                    text: "Here are 2 random phone numbers:".into(),
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
                    text: "\n\n1. (503) 272-8192".into(),
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
                    text: "\n2. (617) 985-3519.".into(),
                }],
                token_count: 0,
                processed_index: 70,
                start_index: 51,
                input_start_index: 10,
                input_end_index: 10,
            },
        ]);
    });

    let mut pii_detector_sentence_server = MockServer::new_http("pii_detector_sentence");
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
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
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
                Message { role: Role::User, content: Some(Content::Text("Can you generate 2 random phone numbers?".into())), ..Default::default()},
            ],
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
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
