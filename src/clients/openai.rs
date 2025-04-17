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

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use http_body_util::BodyExt;
use hyper::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::mpsc;

use super::{
    Client, Error, HttpClient, create_http_client, detector::ContentAnalysisResponse,
    http::HttpClientExt,
};
use crate::{
    config::ServiceConfig,
    health::HealthCheckResult,
    models::{DetectionWarningReason, DetectorParams, ValidationError},
    orchestrator,
};

const DEFAULT_PORT: u16 = 8080;

const CHAT_COMPLETIONS_ENDPOINT: &str = "/v1/chat/completions";

#[derive(Clone)]
pub struct OpenAiClient {
    client: HttpClient,
    health_client: Option<HttpClient>,
}

impl OpenAiClient {
    pub async fn new(
        config: &ServiceConfig,
        health_config: Option<&ServiceConfig>,
    ) -> Result<Self, Error> {
        let client = create_http_client(DEFAULT_PORT, config).await?;
        let health_client = if let Some(health_config) = health_config {
            Some(create_http_client(DEFAULT_PORT, health_config).await?)
        } else {
            None
        };
        Ok(Self {
            client,
            health_client,
        })
    }

    pub fn client(&self) -> &HttpClient {
        &self.client
    }

    pub async fn chat_completions(
        &self,
        request: ChatCompletionsRequest,
        headers: HeaderMap,
    ) -> Result<ChatCompletionsResponse, Error> {
        let request_json = serde_json::to_value(&request).unwrap();
        dbg!(&request_json);
        let url = self.inner().endpoint(CHAT_COMPLETIONS_ENDPOINT);
        if request.stream {
            let (tx, rx) = mpsc::channel(32);
            let mut event_stream = self
                .inner()
                .post(url, headers, request)
                .await?
                .0
                .into_data_stream()
                .eventsource();
            // Spawn task to forward events to receiver
            tokio::spawn(async move {
                while let Some(result) = event_stream.next().await {
                    match result {
                        Ok(event) if event.data == "[DONE]" => {
                            // Send None to signal that the stream completed
                            let _ = tx.send(Ok(None)).await;
                            break;
                        }
                        Ok(event) => match serde_json::from_str::<ChatCompletionChunk>(&event.data)
                        {
                            Ok(chunk) => {
                                let _ = tx.send(Ok(Some(chunk))).await;
                            }
                            Err(e) => {
                                let error = Error::Http {
                                    code: StatusCode::INTERNAL_SERVER_ERROR,
                                    message: format!("deserialization error: {e}"),
                                };
                                let _ = tx.send(Err(error.into())).await;
                            }
                        },
                        Err(error) => {
                            // We received an error from the event stream, send error message
                            let error = Error::Http {
                                code: StatusCode::INTERNAL_SERVER_ERROR,
                                message: error.to_string(),
                            };
                            let _ = tx.send(Err(error.into())).await;
                        }
                    }
                }
            });
            Ok(ChatCompletionsResponse::Streaming(rx))
        } else {
            let response = self.client.clone().post(url, headers, request).await?;
            match response.status() {
                StatusCode::OK => Ok(response.json::<ChatCompletion>().await?.into()),
                _ => {
                    let code = response.status();
                    let message = if let Ok(response) = response.json::<OpenAiError>().await {
                        response.message
                    } else {
                        "unknown error occurred".into()
                    };
                    Err(Error::Http { code, message })
                }
            }
        }
    }
}

#[async_trait]
impl Client for OpenAiClient {
    fn name(&self) -> &str {
        "openai"
    }

    async fn health(&self) -> HealthCheckResult {
        if let Some(health_client) = &self.health_client {
            health_client.health().await
        } else {
            self.client.health().await
        }
    }
}

impl HttpClientExt for OpenAiClient {
    fn inner(&self) -> &HttpClient {
        self.client()
    }
}

#[derive(Debug)]
pub enum ChatCompletionsResponse {
    Unary(Box<ChatCompletion>),
    Streaming(mpsc::Receiver<Result<Option<ChatCompletionChunk>, orchestrator::Error>>),
}

impl From<ChatCompletion> for ChatCompletionsResponse {
    fn from(value: ChatCompletion) -> Self {
        Self::Unary(Box::new(value))
    }
}

/// Represents a chat completions request.
///
/// As orchestrator is only concerned with a limited subset
/// of request fields, we deserialize to an inner [`serde_json::Map`]
/// and only validate and extract the fields used by this service.
/// This type is then serialized to the inner [`serde_json::Map`].
///
/// This is to avoid tracking and updating OpenAI and vLLM
/// parameter additions/changes. Full validation is delegated to
/// the downstream server implementation.
///
/// Validated fields: detectors (internal), model, messages
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(try_from = "Map<String, Value>")]
pub struct ChatCompletionsRequest {
    /// Detector config.
    pub detectors: DetectorConfig,
    /// Stream parameter.
    pub stream: bool,
    /// Model name.
    pub model: String,
    /// Messages.
    pub messages: Vec<Message>,
    /// Inner request.
    pub inner: Map<String, Value>,
}

impl TryFrom<Map<String, Value>> for ChatCompletionsRequest {
    type Error = ValidationError;

    fn try_from(mut value: Map<String, Value>) -> Result<Self, Self::Error> {
        let detectors = if let Some(detectors) = value.remove("detectors") {
            DetectorConfig::deserialize(detectors)
                .map_err(|_| ValidationError::Invalid("error deserializing `detectors`".into()))
        } else {
            Err(ValidationError::Required("detectors".into()))
        }?;
        let stream = value
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or_default();
        let model = if let Some(Value::String(model)) = value.get("model") {
            Ok(model.clone())
        } else {
            Err(ValidationError::Required("model".into()))
        }?;
        if model.is_empty() {
            return Err(ValidationError::Invalid("`model` must not be empty".into()));
        }
        let messages = if let Some(messages) = value.get("messages") {
            Vec::<Message>::deserialize(messages)
                .map_err(|_| ValidationError::Invalid("error deserializing `messages`".into()))
        } else {
            Err(ValidationError::Required("messages".into()))
        }?;
        if messages.is_empty() {
            return Err(ValidationError::Invalid(
                "`messages` must not be empty".into(),
            ));
        }
        Ok(ChatCompletionsRequest {
            detectors,
            stream,
            model,
            messages,
            inner: value,
        })
    }
}

impl Serialize for ChatCompletionsRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.inner.serialize(serializer)
    }
}

/// Structure to contain parameters for detectors.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectorConfig {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub input: HashMap<String, DetectorParams>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub output: HashMap<String, DetectorParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFormat {
    /// The type of response format being defined.
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub json_schema: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// The type of the tool.
    #[serde(rename = "type")]
    pub r#type: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    /// The name of the function to be called.
    pub name: String,
    /// A description of what the function does, used by the model to choose when and how to call the function.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The parameters the functions accepts, described as a JSON Schema object.
    // JSON Schema is not strictly defined here since parameters are passed through
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, serde_json::Value>,
    /// Whether to enable strict schema adherence when generating the function call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    /// `none` means the model will not call any tool and instead generates a message.
    /// `auto` means the model can pick between generating a message or calling one or more tools.
    /// `required` means the model must call one or more tools.
    String,
    /// Specifies a tool the model should use. Use to force the model to call a specific function.
    Object(ToolChoiceObject),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceObject {
    /// The type of the tool.
    #[serde(rename = "type")]
    pub r#type: String,
    pub function: Function,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOptions {
    /// If set, an additional chunk will be streamed before the data: [DONE] message.
    /// The usage field on this chunk shows the token usage statistics for the entire
    /// request, and the choices field will always be an empty array. All other chunks
    /// will also include a usage field, but with a null value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_usage: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    User,
    Developer,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// The role of the author of this message.
    pub role: Role,
    /// The contents of the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    /// An optional name for the participant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The refusal message by the assistant. (assistant message only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
    /// The tool calls generated by the model, such as function calls. (assistant message only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Tool call that this message is responding to. (tool message only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Content {
    /// The text contents of the message.
    Text(String),
    /// Array of content parts.
    Array(Vec<ContentPart>),
}

impl From<String> for Content {
    fn from(value: String) -> Self {
        Content::Text(value)
    }
}

impl From<&str> for Content {
    fn from(value: &str) -> Self {
        Content::Text(value.to_string())
    }
}

impl From<Vec<ContentPart>> for Content {
    fn from(value: Vec<ContentPart>) -> Self {
        Content::Array(value)
    }
}

impl From<String> for ContentPart {
    fn from(value: String) -> Self {
        ContentPart {
            r#type: ContentType::Text,
            text: Some(value),
            image_url: None,
            refusal: None,
        }
    }
}

impl From<Vec<String>> for Content {
    fn from(value: Vec<String>) -> Self {
        Content::Array(value.into_iter().map(|v| v.into()).collect())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub enum ContentType {
    #[serde(rename = "text")]
    #[default]
    Text,
    #[serde(rename = "image_url")]
    ImageUrl,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContentPart {
    /// The type of the content part.
    #[serde(rename = "type")]
    pub r#type: ContentType,
    /// Text content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Image content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<ImageUrl>,
    /// The refusal message generated by the model. (assistant message only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageUrl {
    /// Either a URL of the image or the base64 encoded image data.
    pub url: String,
    /// Specifies the detail level of the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    /// The ID of the tool call.
    pub id: String,
    /// The type of the tool.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The function that the model called.
    pub function: Function,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Function {
    /// The name of the function to call.
    pub name: String,
    /// The arguments to call the function with, as generated by the model in JSON format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// Represents a chat completion response returned by model, based on the provided input.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatCompletion {
    /// A unique identifier for the chat completion.
    pub id: String,
    /// The object type, which is always `chat.completion`.
    pub object: String,
    /// The Unix timestamp (in seconds) of when the chat completion was created.
    pub created: i64,
    /// The model used for the chat completion.
    pub model: String,
    /// A list of chat completion choices. Can be more than one if n is greater than 1.
    pub choices: Vec<ChatCompletionChoice>,
    /// Usage statistics for the completion request.
    pub usage: Usage,
    /// This fingerprint represents the backend configuration that the model runs with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    /// The service tier used for processing the request.
    /// This field is only included if the `service_tier` parameter is specified in the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    /// Detections
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detections: Option<ChatDetections>,
    /// Warnings
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<OrchestratorWarning>,
}

/// A chat completion choice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatCompletionChoice {
    /// The index of the choice in the list of choices.
    pub index: u32,
    /// A chat completion message generated by the model.
    pub message: ChatCompletionMessage,
    /// Log probability information for the choice.
    pub logprobs: Option<ChatCompletionLogprobs>,
    /// The reason the model stopped generating tokens.
    pub finish_reason: String,
}

/// A chat completion message generated by the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatCompletionMessage {
    /// The role of the author of this message.
    pub role: Role,
    /// The contents of the message.
    pub content: Option<String>,
    /// The tool calls generated by the model, such as function calls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// The refusal message generated by the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ChatCompletionLogprobs {
    /// A list of message content tokens with log probability information.
    pub content: Option<Vec<ChatCompletionLogprob>>,
    /// A list of message refusal tokens with log probability information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Vec<ChatCompletionLogprob>>,
}

/// Log probability information for a choice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatCompletionLogprob {
    /// The token.
    pub token: String,
    /// The log probability of this token.
    pub logprob: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    /// List of the most likely tokens and their log probability, at this token position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<Vec<ChatCompletionTopLogprob>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatCompletionTopLogprob {
    /// The token.
    pub token: String,
    /// The log probability of this token.
    pub logprob: f32,
}

/// Represents a streamed chunk of a chat completion response returned by model, based on the provided input.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunk {
    /// A unique identifier for the chat completion. Each chunk has the same ID.
    pub id: String,
    /// The object type, which is always `chat.completion.chunk`.
    pub object: String,
    /// The Unix timestamp (in seconds) of when the chat completion was created. Each chunk has the same timestamp.
    pub created: i64,
    /// The model to generate the completion.
    pub model: String,
    /// This fingerprint represents the backend configuration that the model runs with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    /// A list of chat completion choices.
    pub choices: Vec<ChatCompletionChunkChoice>,
    /// The service tier used for processing the request.
    /// This field is only included if the service_tier parameter is specified in the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Detections
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detections: Option<ChatDetections>,
    /// Warnings
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<OrchestratorWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunkChoice {
    /// The index of the choice in the list of choices.
    pub index: u32,
    /// A chat completion delta generated by streamed model responses.
    pub delta: ChatCompletionDelta,
    /// Log probability information for the choice.
    pub logprobs: Option<ChatCompletionLogprobs>,
    /// The reason the model stopped generating tokens.
    pub finish_reason: Option<String>,
}

/// A chat completion delta generated by streamed model responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionDelta {
    /// The role of the author of this message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    /// The contents of the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The refusal message generated by the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
    /// The tool calls generated by the model, such as function calls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

/// Usage statistics for a completion.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    /// Number of tokens in the prompt.
    pub prompt_tokens: u32,
    /// Total number of tokens used in the request (prompt + completion).
    pub total_tokens: u32,
    /// Number of tokens in the generated completion.
    pub completion_tokens: u32,
    /// Breakdown of tokens used in the prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_token_details: Option<PromptTokenDetails>,
    /// Breakdown of tokens used in a completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_token_details: Option<CompletionTokenDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionTokenDetails {
    pub audio_tokens: u32,
    pub reasoning_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptTokenDetails {
    pub audio_tokens: u32,
    pub cached_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StopTokens {
    Array(Vec<String>),
    String(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiError {
    pub object: Option<String>,
    pub message: String,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
    pub param: Option<String>,
    pub code: u16,
}

/// Guardrails detection results.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatDetections {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<InputDetectionResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output: Vec<OutputDetectionResult>,
}

/// Guardrails detection result for application on input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputDetectionResult {
    pub message_index: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<ContentAnalysisResponse>,
}

/// Guardrails detection result for application output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputDetectionResult {
    pub choice_index: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<ContentAnalysisResponse>,
}

/// Represents the input and output of detection results following processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionResult {
    pub index: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<ContentAnalysisResponse>,
}

/// Warnings generated by guardrails.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrchestratorWarning {
    r#type: DetectionWarningReason,
    message: String,
}

impl OrchestratorWarning {
    pub fn new(warning_type: DetectionWarningReason, message: &str) -> Self {
        Self {
            r#type: warning_type,
            message: message.to_string(),
        }
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_chat_completions_request() -> Result<(), serde_json::Error> {
        // Test deserialize
        let json_request = json!({
            "model": "test",
            "detectors": DetectorConfig::default(),
            "messages": vec![Message {
                content: Some(Content::Text("Hi there!".to_string())),
                ..Default::default()
            }],
        });
        let request = ChatCompletionsRequest::deserialize(&json_request)?;
        let mut inner = json_request.as_object().unwrap().to_owned();
        inner.remove("detectors").unwrap();
        assert_eq!(
            request,
            ChatCompletionsRequest {
                detectors: DetectorConfig::default(),
                stream: false,
                model: "test".into(),
                messages: vec![Message {
                    content: Some(Content::Text("Hi there!".to_string())),
                    ..Default::default()
                }],
                inner,
            }
        );

        // Test deserialize validation errors
        let result = ChatCompletionsRequest::deserialize(json!({
            "model": "test",
            "messages": vec![Message {
                content: Some(Content::Text("Hi there!".to_string())),
                ..Default::default()
            }],
        }));
        assert!(result.is_err_and(|error| error.to_string() == "`detectors` is required"));

        let result = ChatCompletionsRequest::deserialize(json!({
            "detectors": DetectorConfig::default(),
            "messages": vec![Message {
                content: Some(Content::Text("Hi there!".to_string())),
                ..Default::default()
            }],
        }));
        assert!(result.is_err_and(|error| error.to_string() == "`model` is required"));

        let result = ChatCompletionsRequest::deserialize(json!({
            "model": "",
            "detectors": DetectorConfig::default(),
            "messages": Vec::<Message>::default(),
        }));
        assert!(result.is_err_and(|error| error.to_string() == "`model` must not be empty"));

        let result = ChatCompletionsRequest::deserialize(json!({
            "model": "test",
            "detectors": DetectorConfig::default(),
            "messages": Vec::<Message>::default(),
        }));
        assert!(result.is_err_and(|error| error.to_string() == "`messages` must not be empty"));

        let result = ChatCompletionsRequest::deserialize(json!({
            "model": "test",
            "detectors": DetectorConfig::default(),
            "messages": vec!["invalid"],
        }));
        assert!(result.is_err_and(|error| error.to_string() == "error deserializing `messages`"));

        // Test serialize
        let serialized_request = serde_json::to_value(request)?;
        assert_eq!(
            serialized_request,
            json!({
                "model": "test",
                "messages": vec![Message {
                    content: Some(Content::Text("Hi there!".to_string())),
                    role: Role::User,
                    ..Default::default()
                }],
            })
        );

        Ok(())
    }
}
