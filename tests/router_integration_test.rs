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

/// Helper function to create router envelope format with SSE
fn router_sse_envelope(content: &str) -> String {
    let envelope = json!({
        "content": content
    });
    format!("data: {}\n\n", envelope)
}

/// Helper function to create SSE events with router envelope for watsonx responses
fn router_sse_watsonx(
    messages: impl IntoIterator<Item = serde_json::Value>,
) -> impl IntoIterator<Item = String> {
    messages.into_iter().map(|msg| {
        let msg_str = serde_json::to_string(&msg).unwrap();
        router_sse_envelope(&msg_str)
    })
}

#[test(tokio::test)]
async fn test_router_completions_streaming() -> Result<(), anyhow::Error> {
    let model_id = "test-model";
    let mut router_server = MockServer::new_http("router");

    // Simulate router response with watsonx format
    let watsonx_chunks = [
        json!({
            "model_id": model_id,
            "results": [{
                "generated_text": "Hello",
                "generated_token_count": 1,
                "input_token_count": 5,
                "stop_reason": null
            }]
        }),
        json!({
            "model_id": model_id,
            "results": [{
                "generated_text": " world",
                "generated_token_count": 1,
                "input_token_count": 5,
                "stop_reason": null
            }]
        }),
        json!({
            "model_id": model_id,
            "results": [{
                "generated_text": "!",
                "generated_token_count": 1,
                "input_token_count": 5,
                "stop_reason": "eos_token"
            }]
        }),
    ];

    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_STREAM_ENDPOINT);
        then.text_stream(router_sse_watsonx(watsonx_chunks.clone()));
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

    // Mix data events with keepalive (png) events
    let events = vec![
        "event: png\ndata: \n\n".to_string(), // Keepalive
        router_sse_envelope(
            &json!({
                "model_id": model_id,
                "results": [{
                    "generated_text": "Test",
                    "generated_token_count": 1,
                    "input_token_count": 1,
                    "stop_reason": null
                }]
            })
            .to_string(),
        ),
        "event: png\ndata: \n\n".to_string(), // Keepalive
        router_sse_envelope(
            &json!({
                "model_id": model_id,
                "results": [{
                    "generated_text": " response",
                    "generated_token_count": 1,
                    "input_token_count": 1,
                    "stop_reason": "eos_token"
                }]
            })
            .to_string(),
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

    let watsonx_chunks = [
        json!({
            "model_id": model_id,
            "results": [{
                "generated_text": "I'm",
                "generated_token_count": 1,
                "input_token_count": 10,
                "stop_reason": null
            }]
        }),
        json!({
            "model_id": model_id,
            "results": [{
                "generated_text": " doing",
                "generated_token_count": 1,
                "input_token_count": 10,
                "stop_reason": null
            }]
        }),
        json!({
            "model_id": model_id,
            "results": [{
                "generated_text": " well!",
                "generated_token_count": 1,
                "input_token_count": 10,
                "stop_reason": "eos_token"
            }]
        }),
    ];

    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_STREAM_ENDPOINT);
        then.text_stream(router_sse_watsonx(watsonx_chunks.clone()));
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

    // Simulate router sending an error event
    let events = vec![
        router_sse_envelope(
            &json!({
                "model_id": model_id,
                "results": [{
                    "generated_text": "Start",
                    "generated_token_count": 1,
                    "input_token_count": 1,
                    "stop_reason": null
                }]
            })
            .to_string(),
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

    // Include [DONE] signal at the end
    let events = vec![
        router_sse_envelope(
            &json!({
                "model_id": model_id,
                "results": [{
                    "generated_text": "Response",
                    "generated_token_count": 1,
                    "input_token_count": 1,
                    "stop_reason": "eos_token"
                }]
            })
            .to_string(),
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

    // Mock generation response (watsonx format in router envelope)
    let watsonx_response = json!({
        "model_id": model_id,
        "results": [{
            "generated_text": "This is a safe response",
            "generated_token_count": 5,
            "input_token_count": 3,
            "stop_reason": "eos_token"
        }]
    });

    // Router will receive TWO requests:
    // 1. Detector request to /ml/v1-private/router/handle (http_pass mode)
    // 2. Generation request to /ml/v1-private/router/handle-stream (streaming)

    // Mock detector call (unary, http_pass) - returns response directly
    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_ENDPOINT);
        then.json(&detector_response);
    });

    // Mock generation call (streaming)
    router_server.mock(|when, then| {
        when.post().path(ROUTER_HANDLE_STREAM_ENDPOINT);
        then.text_stream(router_sse_watsonx([watsonx_response.clone()]));
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
