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

//! Integration tests for router functionality
//!
//! These tests verify that the orchestrator correctly integrates with the router
//! by using mock HTTP servers to simulate router behavior.

pub mod common;

use fms_guardrails_orchestr8::clients::openai::{
    ChatCompletionChunk, Completion, CompletionChoice,
};
use futures::TryStreamExt;
use mocktail::prelude::*;
use reqwest::StatusCode;
use serde_json::json;
use test_log::test;
use tracing::debug;

use crate::common::orchestrator::{
    ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT, ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT,
    ORCHESTRATOR_CONFIG_FILE_PATH, SseStream, TestOrchestratorServer,
};

/// Router sender endpoint for streaming requests
const ROUTER_HANDLE_STREAM_ENDPOINT: &str = "/ml/v1-private/router/handle-stream";
/// Router sender endpoint for unary requests
const ROUTER_HANDLE_ENDPOINT: &str = "/ml/v1-private/router/handle";

/// Helper function to create SSE events in OpenAI/vLLM format (no router envelope needed)
fn sse_events(
    messages: impl IntoIterator<Item = serde_json::Value>,
) -> impl IntoIterator<Item = String> {
    messages.into_iter().map(|msg| {
        let msg_str = serde_json::to_string(&msg).unwrap();
        format!("data: {}\n\n", msg_str)
    })
}

#[test(tokio::test)]
async fn test_router_completions_streaming() -> Result<(), anyhow::Error> {
    let model_id = "test-model";
    let mut router_server = MockServer::new_http("router");

    // Router now returns vLLM/OpenAI format directly (no watsonx conversion)
    let vllm_chunks = [
        json!({
            "id": "cmpl-test",
            "object": "text_completion",
            "created": 1754506038,
            "model": model_id,
            "choices": [{
                "index": 0,
                "text": "Hello",
                "finish_reason": null
            }]
        }),
        json!({
            "id": "cmpl-test",
            "object": "text_completion",
            "created": 1754506038,
            "model": model_id,
            "choices": [{
                "index": 0,
                "text": " world",
                "finish_reason": null
            }]
        }),
        json!({
            "id": "cmpl-test",
            "object": "text_completion",
            "created": 1754506038,
            "model": model_id,
            "choices": [{
                "index": 0,
                "text": "!",
                "finish_reason": "stop"
            }]
        }),
    ];

    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_STREAM_ENDPOINT);
        then.text_stream(sse_events(vllm_chunks.clone()));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .router_server(&router_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "prompt": "Say hello",
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("Received messages: {messages:#?}");

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].choices[0].text, "Hello");
    assert_eq!(messages[1].choices[0].text, " world");
    assert_eq!(messages[2].choices[0].text, "!");
    assert_eq!(
        messages[2].choices[0].finish_reason,
        Some("stop".to_string())
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_router_completions_with_keepalive() -> Result<(), anyhow::Error> {
    let model_id = "test-model";
    let mut router_server = MockServer::new_http("router");

    // Mix data events with keepalive (png) events - now in vLLM format
    let events = vec![
        "event: png\ndata: \n\n".to_string(), // Keepalive
        format!(
            "data: {}\n\n",
            json!({
                "id": "cmpl-test",
                "object": "text_completion",
                "created": 1754506038,
                "model": model_id,
                "choices": [{
                    "index": 0,
                    "text": "Test",
                    "finish_reason": null
                }]
            })
        ),
        "event: png\ndata: \n\n".to_string(), // Keepalive
        format!(
            "data: {}\n\n",
            json!({
                "id": "cmpl-test",
                "object": "text_completion",
                "created": 1754506038,
                "model": model_id,
                "choices": [{
                    "index": 0,
                    "text": " response",
                    "finish_reason": "stop"
                }]
            })
        ),
    ];

    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_STREAM_ENDPOINT);
        then.text_stream(events);
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .router_server(&router_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "prompt": "Test",
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;

    // Should only get 2 data messages (keepalives filtered out)
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].choices[0].text, "Test");
    assert_eq!(messages[1].choices[0].text, " response");

    Ok(())
}

#[test(tokio::test)]
async fn test_router_chat_completions_streaming() -> Result<(), anyhow::Error> {
    let model_id = "chat-model";
    let mut router_server = MockServer::new_http("router");

    // Router now returns vLLM/OpenAI chat format directly
    let vllm_chunks = [
        json!({
            "id": "chatcmpl-test",
            "object": "chat.completion.chunk",
            "created": 1754506038,
            "model": model_id,
            "choices": [{
                "index": 0,
                "delta": {"content": "I'm"},
                "finish_reason": null
            }]
        }),
        json!({
            "id": "chatcmpl-test",
            "object": "chat.completion.chunk",
            "created": 1754506038,
            "model": model_id,
            "choices": [{
                "index": 0,
                "delta": {"content": " doing"},
                "finish_reason": null
            }]
        }),
        json!({
            "id": "chatcmpl-test",
            "object": "chat.completion.chunk",
            "created": 1754506038,
            "model": model_id,
            "choices": [{
                "index": 0,
                "delta": {"content": " well!"},
                "finish_reason": "stop"
            }]
        }),
    ];

    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_STREAM_ENDPOINT);
        then.text_stream(sse_events(vllm_chunks.clone()));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .router_server(&router_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "messages": [
                {"role": "user", "content": "How are you?"}
            ],
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<ChatCompletionChunk> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("Chat chunks: {messages:#?}");

    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0].choices[0].delta.content,
        Some("I'm".to_string())
    );
    assert_eq!(
        messages[1].choices[0].delta.content,
        Some(" doing".to_string())
    );
    assert_eq!(
        messages[2].choices[0].delta.content,
        Some(" well!".to_string())
    );
    assert_eq!(
        messages[2].choices[0].finish_reason,
        Some("stop".to_string())
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_router_error_event_handling() -> Result<(), anyhow::Error> {
    let model_id = "test-model";
    let mut router_server = MockServer::new_http("router");

    // Simulate router sending an error event - now in vLLM format
    let events = vec![
        format!(
            "data: {}\n\n",
            json!({
                "id": "cmpl-test",
                "object": "text_completion",
                "created": 1754506038,
                "model": model_id,
                "choices": [{
                    "index": 0,
                    "text": "Start",
                    "finish_reason": null
                }]
            })
        ),
        "event: err\ndata: Router internal error\n\n".to_string(),
    ];

    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_STREAM_ENDPOINT);
        then.text_stream(events);
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .router_server(&router_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "prompt": "Test",
        }))
        .send()
        .await?;

    // Should still get OK status initially
    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let result = sse_stream.try_collect::<Vec<_>>().await;

    // Should get an error due to the err event
    assert!(result.is_err(), "Expected error from err event");

    Ok(())
}

#[test(tokio::test)]
async fn test_router_unary_completions() -> Result<(), anyhow::Error> {
    let model_id = "test-model";
    let mut router_server = MockServer::new_http("router");

    // For unary (non-streaming), router uses http_pass mode which forwards to vLLM
    // and returns vLLM's response directly (OpenAI format, not watsonx)
    let vllm_response = Completion {
        id: "cmpl-test".into(),
        model: model_id.to_string(),
        created: 1754506038,
        object: "text_completion".into(),
        choices: vec![CompletionChoice {
            index: 0,
            text: "Complete response".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        }],
        ..Default::default()
    };

    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_ENDPOINT);
        then.json(&vllm_response);
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .router_server(&router_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": false,
            "model": model_id,
            "prompt": "Test prompt",
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let completion: Completion = response.json().await?;
    debug!("Unary completion: {completion:#?}");

    assert_eq!(completion.model, model_id);
    assert_eq!(completion.choices.len(), 1);
    assert_eq!(completion.choices[0].text, "Complete response");
    assert_eq!(
        completion.choices[0].finish_reason,
        Some("stop".to_string())
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_router_done_signal() -> Result<(), anyhow::Error> {
    let model_id = "test-model";
    let mut router_server = MockServer::new_http("router");

    // Include [DONE] signal at the end - now in vLLM format
    let events = vec![
        format!(
            "data: {}\n\n",
            json!({
                "id": "cmpl-test",
                "object": "text_completion",
                "created": 1754506038,
                "model": model_id,
                "choices": [{
                    "index": 0,
                    "text": "Response",
                    "finish_reason": "stop"
                }]
            })
        ),
        "data: [DONE]\n\n".to_string(),
    ];

    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_STREAM_ENDPOINT);
        then.text_stream(events);
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .router_server(&router_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "prompt": "Test",
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;

    // Should get 1 message before [DONE]
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].choices[0].text, "Response");

    Ok(())
}

// Made with Bob

#[test(tokio::test)]
async fn test_router_with_detectors() -> Result<(), anyhow::Error> {
    let model_id = "test-model";
    let mut router_server = MockServer::new_http("router");

    // Detector response - router returns this directly (no envelope for http_pass)
    // ContentAnalysisResponse requires: start, end, text, detection, detection_type, score
    let detector_response = json!([[{
        "start": 0,
        "end": 11,
        "text": "Test prompt",
        "detection": "safe",
        "detection_type": "content",
        "score": 0.1,
        "evidence": []
    }]]);

    // Mock generation response - now in vLLM/OpenAI format
    let vllm_response = json!({
        "id": "cmpl-test",
        "object": "text_completion",
        "created": 1754506038,
        "model": model_id,
        "choices": [{
            "index": 0,
            "text": "This is a safe response",
            "finish_reason": "stop"
        }]
    });

    // Router will receive TWO requests:
    // 1. Detector request to /ml/v1-private/router/handle (http_pass mode)
    // 2. Generation request to /ml/v1-private/router/handle-stream (http_pass_stream mode)

    // Mock detector call (unary, http_pass) - returns response directly
    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_ENDPOINT);
        then.json(&detector_response);
    });

    // Mock generation call (streaming, http_pass_stream) - returns vLLM format directly
    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_STREAM_ENDPOINT);
        then.text_stream(sse_events([vllm_response.clone()]));
    });

    let test_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .router_server(&router_server)
        .build()
        .await?;

    let response = test_server
        .post(ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT)
        .json(&json!({
            "stream": true,
            "model": model_id,
            "prompt": "Test prompt",
            "detectors": {
                "input": {
                    "pii_detector_whole_doc": {}
                }
            }
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let sse_stream: SseStream<Completion> = SseStream::new(response.bytes_stream());
    let messages = sse_stream.try_collect::<Vec<_>>().await?;
    debug!("Messages with detector: {messages:#?}");

    // Verify we got the generation response
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].choices[0].text, "This is a safe response");
    assert_eq!(
        messages[0].choices[0].finish_reason,
        Some("stop".to_string())
    );

    debug!("Detections present: {:?}", messages[0].detections.is_some());

    Ok(())
}
