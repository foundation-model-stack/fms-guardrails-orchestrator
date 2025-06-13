pub mod common;
use common::orchestrator::*;
use fms_guardrails_orchestr8::clients::openai::{
    ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionDelta, Content, Message, Role,
};
use futures::TryStreamExt;
use mocktail::prelude::*;
use serde::Serialize;
use serde_json::json;
use test_log::test;
use tracing::debug;

// Validate passthrough scenario
#[test(tokio::test)]
async fn no_detectors() -> Result<(), anyhow::Error> {
    let mut chat_completions_server = MockServer::new("chat_completions");
    chat_completions_server.mock(|when, then| {
        when.post().path("/v1/chat/completions").json(
            json!({
                "stream": true,
                "model": "test-0B",
                "messages": [
                    Message { role: Role::Assistant, content: Some(Content::Text("You are a helpful assistant.".into())), ..Default::default() },
                    Message { role: Role::User, content: Some(Content::Text("Hey".into())), ..Default::default() },
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

/// Converts an iterator of serializable messages into an iterator of SSE data messages.
fn sse(messages: impl IntoIterator<Item = impl Serialize>) -> impl IntoIterator<Item = String> {
    messages.into_iter().map(|msg| {
        let msg = serde_json::to_string(&msg).unwrap();
        format!("data: {msg}\n\n")
    })
}
