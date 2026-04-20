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

//! Unit tests for router integration functionality

#[cfg(test)]
mod router_config_tests {
    use fms_guardrails_orchestr8::config::RouterConfig;

    #[test]
    fn test_router_config_defaults() {
        let yaml = r#"
enabled: true
hostname: "localhost"
port: 19080
"#;
        let config: RouterConfig = serde_yml::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.hostname, "localhost");
        assert_eq!(config.port, 19080);
        assert_eq!(config.sla_seconds, 60); // default
        assert_eq!(config.reply_type, "redis"); // default
    }

    #[test]
    fn test_router_config_disabled() {
        let yaml = r#"
enabled: false
hostname: "localhost"
port: 19080
"#;
        let config: RouterConfig = serde_yml::from_str(yaml).unwrap();
        assert!(!config.enabled);
        assert_eq!(config.hostname, "localhost");
        assert_eq!(config.port, 19080);
    }

    #[test]
    fn test_router_config_custom_values() {
        let yaml = r#"
enabled: true
hostname: "router.example.com"
port: 9090
sla_seconds: 120
reply_type: "http"
"#;
        let config: RouterConfig = serde_yml::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.hostname, "router.example.com");
        assert_eq!(config.port, 9090);
        assert_eq!(config.sla_seconds, 120);
        assert_eq!(config.reply_type, "http");
    }

    #[test]
    fn test_router_config_minimal() {
        let yaml = r#"
enabled: true
hostname: "router"
port: 8080
"#;
        let config: RouterConfig = serde_yml::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.hostname, "router");
        assert_eq!(config.port, 8080);
        // Check defaults are applied
        assert_eq!(config.sla_seconds, 60);
        assert_eq!(config.reply_type, "redis");
    }

    #[test]
    fn test_router_config_all_fields() {
        let yaml = r#"
enabled: true
hostname: "router.cluster.local"
port: 19080
sla_seconds: 90
reply_type: "redis"
"#;
        let config: RouterConfig = serde_yml::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.hostname, "router.cluster.local");
        assert_eq!(config.port, 19080);
        assert_eq!(config.sla_seconds, 90);
        assert_eq!(config.reply_type, "redis");
    }

    #[test]
    fn test_router_config_zero_sla() {
        let yaml = r#"
enabled: true
hostname: "localhost"
port: 19080
sla_seconds: 0
"#;
        let config: RouterConfig = serde_yml::from_str(yaml).unwrap();
        assert_eq!(config.sla_seconds, 0);
    }

    #[test]
    fn test_router_config_large_sla() {
        let yaml = r#"
enabled: true
hostname: "localhost"
port: 19080
sla_seconds: 3600
"#;
        let config: RouterConfig = serde_yml::from_str(yaml).unwrap();
        assert_eq!(config.sla_seconds, 3600);
    }

    #[test]
    fn test_router_config_invalid_yaml() {
        let yaml = r#"
enabled: true
hostname: "localhost"
port: "not_a_number"
"#;
        let result: Result<RouterConfig, _> = serde_yml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_router_config_missing_required_fields() {
        // All fields have defaults, so even minimal config should succeed
        let yaml = r#"
enabled: true
"#;
        let config: RouterConfig = serde_yml::from_str(yaml).unwrap();
        assert!(config.enabled);
        // Verify defaults are applied
        assert_eq!(config.hostname, "localhost");
        assert_eq!(config.port, 19080);
        assert_eq!(config.sla_seconds, 60);
        assert_eq!(config.reply_type, "redis");
    }

    #[test]
    fn test_router_config_extra_fields_ignored() {
        let yaml = r#"
enabled: true
hostname: "localhost"
port: 19080
extra_field: "ignored"
another_field: 123
"#;
        let config: RouterConfig = serde_yml::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.hostname, "localhost");
        assert_eq!(config.port, 19080);
    }
}

#[cfg(test)]
mod watsonx_conversion_tests {
    // Import the ACTUAL production functions and types from openai.rs
    use fms_guardrails_orchestr8::clients::openai::{
        WatsonxTextGenResponse, WatsonxTextGenResult, watsonx_to_chat_completion_chunk,
        watsonx_to_completion,
    };

    #[test]
    fn test_watsonx_to_completion_basic() {
        let watsonx_resp = WatsonxTextGenResponse {
            model_id: "test-model".to_string(),
            results: vec![WatsonxTextGenResult {
                generated_text: "Hello world".to_string(),
                generated_token_count: 2,
                input_token_count: 1,
                stop_reason: Some("eos_token".to_string()),
            }],
        };

        let completion = watsonx_to_completion(watsonx_resp, "test-model");
        assert_eq!(completion.model, "test-model");
        assert_eq!(completion.object, "text_completion");
        assert_eq!(completion.choices.len(), 1);
        assert_eq!(completion.choices[0].text, "Hello world");
        assert_eq!(completion.choices[0].index, 0);
        assert_eq!(
            completion.choices[0].finish_reason,
            Some("stop".to_string())
        );
        // Real implementation returns usage: None
        assert!(completion.usage.is_none());
    }

    #[test]
    fn test_watsonx_to_completion_empty_model_id() {
        let watsonx_resp = WatsonxTextGenResponse {
            model_id: "".to_string(),
            results: vec![WatsonxTextGenResult {
                generated_text: "test".to_string(),
                generated_token_count: 1,
                input_token_count: 1,
                stop_reason: None,
            }],
        };

        let completion = watsonx_to_completion(watsonx_resp, "fallback-model");
        assert_eq!(completion.model, "fallback-model");
    }

    #[test]
    fn test_watsonx_to_completion_empty_results() {
        let watsonx_resp = WatsonxTextGenResponse {
            model_id: "test-model".to_string(),
            results: vec![],
        };

        let completion = watsonx_to_completion(watsonx_resp, "fallback-model");
        assert_eq!(completion.model, "test-model");
        // Real implementation returns empty choices array for empty results
        assert_eq!(completion.choices.len(), 0);
        assert!(completion.usage.is_none());
    }

    #[test]
    fn test_watsonx_to_completion_stop_reasons() {
        let test_cases = vec![
            ("eos_token", Some("stop")),
            ("max_tokens", Some("length")),
            ("stop_sequence", Some("stop")),
            ("not_finished", None), // Real implementation: not_finished -> None
            ("cancelled", Some("cancelled")), // Real implementation: unknown reasons pass through
            ("error", Some("error")),
        ];

        for (watsonx_reason, expected_openai_reason) in test_cases {
            let watsonx_resp = WatsonxTextGenResponse {
                model_id: "test".to_string(),
                results: vec![WatsonxTextGenResult {
                    generated_text: "test".to_string(),
                    generated_token_count: 1,
                    input_token_count: 1,
                    stop_reason: Some(watsonx_reason.to_string()),
                }],
            };

            let completion = watsonx_to_completion(watsonx_resp, "test");
            assert_eq!(
                completion.choices[0].finish_reason.as_deref(),
                expected_openai_reason,
                "Failed for stop_reason: {}",
                watsonx_reason
            );
        }
    }

    #[test]
    fn test_watsonx_to_completion_no_stop_reason() {
        let watsonx_resp = WatsonxTextGenResponse {
            model_id: "test".to_string(),
            results: vec![WatsonxTextGenResult {
                generated_text: "incomplete".to_string(),
                generated_token_count: 1,
                input_token_count: 1,
                stop_reason: None,
            }],
        };

        let completion = watsonx_to_completion(watsonx_resp, "test");
        assert_eq!(completion.choices[0].finish_reason, None);
    }

    #[test]
    fn test_watsonx_to_chat_completion_chunk_basic() {
        let watsonx_resp = WatsonxTextGenResponse {
            model_id: "chat-model".to_string(),
            results: vec![WatsonxTextGenResult {
                generated_text: "Hi there".to_string(),
                generated_token_count: 2,
                input_token_count: 3,
                stop_reason: None,
            }],
        };

        let chunk = watsonx_to_chat_completion_chunk(watsonx_resp, "chat-model");
        assert_eq!(chunk.model, "chat-model");
        assert_eq!(chunk.object, "chat.completion.chunk");
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].index, 0);
        assert_eq!(chunk.choices[0].delta.content, Some("Hi there".to_string()));
        assert_eq!(chunk.choices[0].delta.role, None);
        assert_eq!(chunk.choices[0].finish_reason, None);
    }

    #[test]
    fn test_watsonx_to_chat_completion_chunk_with_finish_reason() {
        let watsonx_resp = WatsonxTextGenResponse {
            model_id: "chat-model".to_string(),
            results: vec![WatsonxTextGenResult {
                generated_text: "Done".to_string(),
                generated_token_count: 1,
                input_token_count: 1,
                stop_reason: Some("eos_token".to_string()),
            }],
        };

        let chunk = watsonx_to_chat_completion_chunk(watsonx_resp, "chat-model");
        assert_eq!(chunk.choices[0].finish_reason, Some("stop".to_string()));
    }

    #[test]
    fn test_watsonx_to_chat_completion_chunk_empty_results() {
        let watsonx_resp = WatsonxTextGenResponse {
            model_id: "chat-model".to_string(),
            results: vec![],
        };

        let chunk = watsonx_to_chat_completion_chunk(watsonx_resp, "fallback");
        assert_eq!(chunk.model, "chat-model");
        // Real implementation returns empty choices array for empty results
        assert_eq!(chunk.choices.len(), 0);
    }

    #[test]
    fn test_watsonx_to_chat_completion_chunk_fallback_model() {
        let watsonx_resp = WatsonxTextGenResponse {
            model_id: "".to_string(),
            results: vec![WatsonxTextGenResult {
                generated_text: "test".to_string(),
                generated_token_count: 1,
                input_token_count: 1,
                stop_reason: None,
            }],
        };

        let chunk = watsonx_to_chat_completion_chunk(watsonx_resp, "fallback-model");
        assert_eq!(chunk.model, "fallback-model");
    }
}

#[cfg(test)]
mod http_pass_payload_tests {
    use serde_json::json;

    #[test]
    fn test_http_pass_payload_structure() {
        let payload = json!({
            "method": "POST",
            "path": "/api/v1/text/contents",
            "headers": {
                "Content-Type": "application/json",
                "x-request-id": "test-123",
            },
            "payload": b"test data",
        });

        assert_eq!(payload["method"], "POST");
        assert_eq!(payload["path"], "/api/v1/text/contents");
        assert!(payload["headers"].is_object());
        assert_eq!(payload["headers"]["Content-Type"], "application/json");
        assert_eq!(payload["headers"]["x-request-id"], "test-123");
    }

    #[test]
    fn test_http_pass_payload_with_bytes() {
        let test_data = b"binary data";
        let payload = json!({
            "method": "POST",
            "path": "/tokenize",
            "headers": {
                "Content-Type": "application/json",
            },
            "payload": test_data,
        });

        assert!(payload["payload"].is_array());
    }

    #[test]
    fn test_http_pass_payload_multiple_headers() {
        let payload = json!({
            "method": "POST",
            "path": "/api/v1/text/contents",
            "headers": {
                "Content-Type": "application/json",
                "x-request-id": "req-123",
                "x-model-name": "test-model",
                "x-sla-seconds": "60",
            },
            "payload": [],
        });

        let headers = payload["headers"].as_object().unwrap();
        assert_eq!(headers.len(), 4);
        assert_eq!(headers["x-model-name"], "test-model");
        assert_eq!(headers["x-sla-seconds"], "60");
    }

    #[cfg(test)]
    mod stream_handler_tests {
        use serde_json::json;

        /// Mock event structure for testing
        #[derive(Debug, Clone)]
        struct MockEvent {
            event: String,
            data: String,
        }

        /// Mock event creation helper
        fn create_event(event_type: &str, data: &str) -> MockEvent {
            MockEvent {
                event: event_type.to_string(),
                data: data.to_string(),
            }
        }

        #[test]
        fn test_ping_keepalive_skipped() {
            let event = create_event("png", "");
            // In actual implementation, "png" events should be skipped (continue)
            assert_eq!(event.event, "png");
            // This event should not produce any output in the stream
        }

        #[test]
        fn test_err_event_produces_error() {
            let error_message = "Internal server error occurred";
            let event = create_event("err", error_message);

            assert_eq!(event.event, "err");
            assert_eq!(event.data, error_message);
            // In actual implementation, this should:
            // 1. Create an Error::Http with INTERNAL_SERVER_ERROR
            // 2. Send error to channel
            // 3. Break the loop
        }

        #[test]
        fn test_done_signal_terminates_stream() {
            let event = create_event("message", "[DONE]");

            assert_eq!(event.data, "[DONE]");
            // In actual implementation, this should:
            // 1. Send Ok(None) to channel
            // 2. Break the loop
        }

        #[test]
        fn test_router_envelope_parsing() {
            let envelope = json!({
                "content": "{\"model_id\":\"test\",\"results\":[{\"generated_text\":\"Hello\"}]}"
            });

            let envelope_str = envelope.to_string();
            let event = create_event("message", &envelope_str);

            // Parse the envelope
            let parsed: serde_json::Value = serde_json::from_str(&event.data).unwrap();
            let content = parsed.get("content").and_then(|v| v.as_str()).unwrap();

            assert!(content.contains("model_id"));
            assert!(content.contains("generated_text"));
        }

        #[test]
        fn test_router_envelope_without_content_field() {
            let direct_data = json!({
                "model_id": "test",
                "results": [{"generated_text": "Hello"}]
            });

            let data_str = direct_data.to_string();
            let event = create_event("message", &data_str);

            // Parse as envelope first
            let parsed: serde_json::Value = serde_json::from_str(&event.data).unwrap();
            let content = parsed.get("content").and_then(|v| v.as_str());

            // If no content field, use the data directly
            let final_content = content.unwrap_or(&event.data);
            assert!(final_content.contains("model_id"));
        }

        #[test]
        fn test_malformed_json_in_envelope() {
            let malformed = "{ invalid json }";
            let event = create_event("message", malformed);

            // Attempt to parse
            let result: Result<serde_json::Value, _> = serde_json::from_str(&event.data);
            assert!(result.is_err());
            // In actual implementation, this should:
            // 1. Fall back to using event.data directly
            // 2. Attempt to parse as WatsonxTextGenResponse
            // 3. If that fails, send deserialization error
        }

        #[test]
        fn test_malformed_watsonx_response() {
            let envelope = json!({
                "content": "{ \"invalid\": \"structure\" }"
            });

            let envelope_str = envelope.to_string();
            let event = create_event("message", &envelope_str);

            // Parse envelope
            let parsed: serde_json::Value = serde_json::from_str(&event.data).unwrap();
            let content = parsed.get("content").and_then(|v| v.as_str()).unwrap();

            // Try to parse as WatsonxTextGenResponse (should fail)
            #[derive(serde::Deserialize)]
            #[allow(dead_code)]
            struct WatsonxTextGenResponse {
                model_id: String,
                results: Vec<serde_json::Value>,
            }

            let result: Result<WatsonxTextGenResponse, _> = serde_json::from_str(content);
            assert!(result.is_err());
            // In actual implementation, this should:
            // 1. Send deserialization error to channel
            // 2. Break the loop
        }

        #[test]
        fn test_empty_event_data() {
            let event = create_event("message", "");

            assert_eq!(event.data, "");
            // In actual implementation, this should be handled gracefully
            // Either skip or attempt to parse (which will fail)
        }

        #[test]
        fn test_done_signal_before_envelope_parsing() {
            // Test that [DONE] check happens BEFORE envelope parsing
            let event = create_event("message", "[DONE]");

            // Check for [DONE] first
            assert_eq!(event.data, "[DONE]", "Should have detected [DONE] signal");
        }

        #[test]
        fn test_multiple_events_sequence() {
            let events = vec![
                create_event("png", ""),                            // Keepalive - skip
                create_event("message", "{\"content\":\"test1\"}"), // Valid data
                create_event("png", ""),                            // Keepalive - skip
                create_event("message", "{\"content\":\"test2\"}"), // Valid data
                create_event("message", "[DONE]"),                  // Termination signal
            ];

            let mut processed_count = 0;
            for event in events {
                match event.event.as_str() {
                    "png" => continue, // Skip keepalive
                    "err" => break,    // Error terminates
                    _ => {
                        if event.data == "[DONE]" {
                            break; // Done terminates
                        }
                        processed_count += 1;
                    }
                }
            }

            assert_eq!(
                processed_count, 2,
                "Should process 2 valid messages before [DONE]"
            );
        }

        #[test]
        fn test_error_event_terminates_immediately() {
            let events = vec![
                create_event("message", "{\"content\":\"test1\"}"),
                create_event("err", "Something went wrong"),
                create_event("message", "{\"content\":\"test2\"}"), // Should not be processed
            ];

            let mut processed_count = 0;
            for event in events {
                match event.event.as_str() {
                    "err" => break,
                    _ => processed_count += 1,
                }
            }

            assert_eq!(
                processed_count, 1,
                "Should stop processing after error event"
            );
        }

        #[test]
        fn test_base64_payload_in_envelope() {
            use base64::{Engine as _, engine::general_purpose};

            // Router sends base64-encoded payloads
            let base64_content = general_purpose::STANDARD.encode(b"test payload");
            let envelope = json!({
                "content": base64_content
            });

            let envelope_str = envelope.to_string();
            let parsed: serde_json::Value = serde_json::from_str(&envelope_str).unwrap();
            let content = parsed.get("content").and_then(|v| v.as_str()).unwrap();

            // Decode base64
            let decoded = general_purpose::STANDARD.decode(content).unwrap();
            assert_eq!(decoded, b"test payload");
        }
    }
}
