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
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use url::Url;

use super::{
    Client, Error, HttpClient, create_http_client,
    detector::ContentAnalysisResponse,
    http::{HttpClientExt, RequestBody},
};
use crate::{
    config::ServiceConfig,
    health::HealthCheckResult,
    models::{DetectionWarningReason, DetectorParams, ValidationError},
    orchestrator,
};

const DEFAULT_PORT: u16 = 8080;

const CHAT_COMPLETIONS_ENDPOINT: &str = "/v1/chat/completions";
const COMPLETIONS_ENDPOINT: &str = "/v1/completions";

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
        let url = self.client.endpoint(CHAT_COMPLETIONS_ENDPOINT);
        if let Some(true) = request.stream {
            let rx = self.handle_streaming(url, request, headers).await?;
            Ok(ChatCompletionsResponse::Streaming(rx))
        } else {
            let chat_completion = self.handle_unary(url, request, headers).await?;
            Ok(ChatCompletionsResponse::Unary(chat_completion))
        }
    }

    pub async fn completions(
        &self,
        request: CompletionsRequest,
        headers: HeaderMap,
    ) -> Result<CompletionsResponse, Error> {
        let url = self.client.endpoint(COMPLETIONS_ENDPOINT);
        if let Some(true) = request.stream {
            let rx = self.handle_streaming(url, request, headers).await?;
            Ok(CompletionsResponse::Streaming(rx))
        } else {
            let completion = self.handle_unary(url, request, headers).await?;
            Ok(CompletionsResponse::Unary(completion))
        }
    }

    async fn handle_unary<R, S>(&self, url: Url, request: R, headers: HeaderMap) -> Result<S, Error>
    where
        R: RequestBody,
        S: DeserializeOwned,
    {
        let response = self.client.post(url, headers, request).await?;
        match response.status() {
            StatusCode::OK => response.json::<S>().await,
            _ => {
                // Return error with code and message from downstream server
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

    async fn handle_streaming<R, S>(
        &self,
        url: Url,
        request: R,
        headers: HeaderMap,
    ) -> Result<mpsc::Receiver<Result<Option<S>, orchestrator::Error>>, Error>
    where
        R: RequestBody,
        S: DeserializeOwned + Send + 'static,
    {
        let (tx, rx) = mpsc::channel(32);
        let response = self.client.post(url, headers, request).await?;
        match response.status() {
            StatusCode::OK => {
                // Create event stream
                let mut event_stream = response.0.into_data_stream().eventsource();
                // Spawn task to consume event stream and send messages to receiver
                tokio::spawn(async move {
                    while let Some(result) = event_stream.next().await {
                        match result {
                            Ok(event) if event.data == "[DONE]" => {
                                // DONE message: send None to signal completion
                                let _ = tx.send(Ok(None)).await;
                                break;
                            }
                            // Attempt to deserialize to S
                            Ok(event) => match serde_json::from_str::<S>(&event.data) {
                                Ok(message) => {
                                    let _ = tx.send(Ok(Some(message))).await;
                                }
                                Err(_serde_error) => {
                                    // Failed to deserialize to S, attempt to deserialize to OpenAiErrorMessage
                                    let error = match serde_json::from_str::<OpenAiErrorMessage>(
                                        &event.data,
                                    ) {
                                        // Return error with code and message from downstream server
                                        Ok(openai_error) => Error::Http {
                                            code: StatusCode::from_u16(openai_error.error.code)
                                                .unwrap(),
                                            message: openai_error.error.message,
                                        },
                                        // Failed to deserialize to S and OpenAiErrorMessage
                                        // Return internal server error
                                        Err(serde_error) => Error::Http {
                                            code: StatusCode::INTERNAL_SERVER_ERROR,
                                            message: format!(
                                                "deserialization error: {serde_error}"
                                            ),
                                        },
                                    };
                                    let _ = tx.send(Err(error.into())).await;
                                }
                            },
                            Err(error) => {
                                // Event stream error
                                // Return internal server error
                                let error = Error::Http {
                                    code: StatusCode::INTERNAL_SERVER_ERROR,
                                    message: error.to_string(),
                                };
                                let _ = tx.send(Err(error.into())).await;
                            }
                        }
                    }
                });
                Ok(rx)
            }
            _ => {
                // Return error with code and message from downstream server
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

/// Chat completions response.
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

/// Completions (legacy) response.
#[derive(Debug)]
pub enum CompletionsResponse {
    Unary(Box<Completion>),
    Streaming(mpsc::Receiver<Result<Option<Completion>, orchestrator::Error>>),
}

impl From<Completion> for CompletionsResponse {
    fn from(value: Completion) -> Self {
        Self::Unary(Box::new(value))
    }
}

/// Chat completions request.
///
/// As orchestrator is only concerned with a limited subset
/// of request fields, we only inline and validate fields used by
/// this service. Extra fields are deserialized to `extra` via
/// struct flattening. The `detectors` field is not serialized.
///
/// This is to avoid tracking and updating OpenAI and vLLM
/// parameter additions/changes. Full validation is delegated to
/// the downstream server implementation.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatCompletionsRequest {
    /// Detector config.
    #[serde(default, skip_serializing)]
    pub detectors: DetectorConfig,
    /// Stream parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Model name.
    pub model: String,
    /// Messages.
    pub messages: Vec<Message>,
    /// Extra fields not captured above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ChatCompletionsRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.model.is_empty() {
            return Err(ValidationError::Invalid("`model` must not be empty".into()));
        }
        if self.messages.is_empty() {
            return Err(ValidationError::Invalid(
                "`messages` must not be empty".into(),
            ));
        }

        if !self.detectors.input.is_empty() {
            // Content of type Array is not supported yet
            // Adding this validation separately as we do plan to support arrays of string in the future
            if let Some(Content::Array(_)) = self.messages.last().unwrap().content {
                return Err(ValidationError::Invalid(
                    "Detection on array is not supported".into(),
                ));
            }

            // As text_content detections only run on last message at the moment, only the last
            // message is being validated.
            if self.messages.last().unwrap().is_text_content_empty() {
                return Err(ValidationError::Invalid(
                    "if input detectors are provided, `content` must not be empty on last message"
                        .into(),
                ));
            }
        }

        Ok(())
    }
}

/// Completions (legacy) request.
///
/// As orchestrator is only concerned with a limited subset
/// of request fields, we only inline and validate fields used by
/// this service. Extra fields are deserialized to `extra` via
/// struct flattening. The `detectors` field is not serialized.
///
/// This is to avoid tracking and updating OpenAI and vLLM
/// parameter additions/changes. Full validation is delegated to
/// the downstream server implementation.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionsRequest {
    /// Detector config.
    #[serde(default, skip_serializing)]
    pub detectors: DetectorConfig,
    /// Stream parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Model name.
    pub model: String,
    /// Prompt text.
    pub prompt: String,
    /// Extra fields not captured above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl CompletionsRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.model.is_empty() {
            return Err(ValidationError::Invalid("`model` must not be empty".into()));
        }
        if self.prompt.is_empty() {
            return Err(ValidationError::Invalid(
                "`prompt` must not be empty".into(),
            ));
        }
        Ok(())
    }
}

/// Detector config.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectorConfig {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub input: HashMap<String, DetectorParams>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub output: HashMap<String, DetectorParams>,
}

/// Response format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFormat {
    /// The type of response format being defined.
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub json_schema: HashMap<String, serde_json::Value>,
}

/// Tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// The type of the tool.
    #[serde(rename = "type")]
    pub r#type: String,
    pub function: ToolFunction,
}

/// Tool function.
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

/// Tool choice.
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

/// Tool choice object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceObject {
    /// The type of the tool.
    #[serde(rename = "type")]
    pub r#type: String,
    pub function: FunctionCall,
}

/// Stream options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOptions {
    /// If set, an additional chunk will be streamed before the data: [DONE] message.
    /// The usage field on this chunk shows the token usage statistics for the entire
    /// request, and the choices field will always be an empty array. All other chunks
    /// will also include a usage field, but with a null value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_usage: Option<bool>,
}

/// Role.
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

/// Message.
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

impl Message {
    /// Checks if text content of a message is empty.
    ///
    /// The following messages are considered empty:
    /// 1. [`Message::content`] is None.
    /// 2. [`Message::content`] is an empty string.
    /// 3. [`Message::content`] is an empty array.
    /// 4. [`Message::content`] is an array of empty strings and ContentType is Text.
    pub fn is_text_content_empty(&self) -> bool {
        match &self.content {
            Some(content) => match content {
                Content::Text(string) => string.is_empty(),
                Content::Array(content_parts) => {
                    content_parts.is_empty()
                        || content_parts.iter().all(|content_part| {
                            content_part.text.is_none()
                                || (content_part.r#type == ContentType::Text
                                    && content_part.text.as_ref().unwrap().is_empty())
                        })
                }
            },
            None => true,
        }
    }
}

/// Content.
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

/// Content type.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub enum ContentType {
    #[serde(rename = "text")]
    #[default]
    Text,
    #[serde(rename = "image_url")]
    ImageUrl,
}

/// Content part.
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

/// Image url.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageUrl {
    /// Either a URL of the image or the base64 encoded image data.
    pub url: String,
    /// Specifies the detail level of the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    /// Index
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    /// The ID of the tool call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The type of the tool.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// The function that the model called.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionCall>,
}

/// Function call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCall {
    /// The name of the function to call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The arguments to call the function with, as generated by the model in JSON format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// Chat completion response.
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
    /// Prompt logprobs.
    pub prompt_logprobs: Option<Vec<Option<HashMap<String, Logprob>>>>,
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

/// Chat completion choice.
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
    /// The stop string or token id that caused the completion.
    pub stop_reason: Option<String>,
}

/// Chat completion message.
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

/// Chat completion logprobs.
#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct ChatCompletionLogprobs {
    /// A list of message content tokens with log probability information.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ChatCompletionLogprob>,
    /// A list of message refusal tokens with log probability information.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refusal: Vec<ChatCompletionLogprob>,
}

/// Chat completion logprob.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatCompletionLogprob {
    /// The token.
    pub token: String,
    /// The log probability of this token.
    pub logprob: f32,
    /// A list of integers representing the UTF-8 bytes representation of the token.
    pub bytes: Option<Vec<u8>>,
    /// List of the most likely tokens and their log probability, at this token position.
    pub top_logprobs: Option<Vec<ChatCompletionTopLogprob>>,
}

/// Chat completion top logprob.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatCompletionTopLogprob {
    /// The token.
    pub token: String,
    /// The log probability of this token.
    pub logprob: f32,
    /// A list of integers representing the UTF-8 bytes representation of the token.
    pub bytes: Option<Vec<u8>>,
}

/// Streaming chat completion chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Default for ChatCompletionChunk {
    fn default() -> Self {
        Self {
            id: Default::default(),
            object: "chat.completion.chunk".into(),
            created: Default::default(),
            model: Default::default(),
            system_fingerprint: Default::default(),
            choices: Default::default(),
            service_tier: Default::default(),
            usage: Default::default(),
            detections: Default::default(),
            warnings: Default::default(),
        }
    }
}

/// Streaming chat completion chunk choice.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunkChoice {
    /// The index of the choice in the list of choices.
    pub index: u32,
    /// A chat completion delta generated by streamed model responses.
    pub delta: ChatCompletionDelta,
    /// Log probability information for the choice.
    pub logprobs: Option<ChatCompletionLogprobs>,
    /// The reason the model stopped generating tokens.
    pub finish_reason: Option<String>,
    /// The stop string or token id that caused the completion.
    pub stop_reason: Option<String>,
}

/// Streaming chat completion delta.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
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

/// Completion (legacy) response. Also used for streaming.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Completion {
    /// A unique identifier for the completion.
    pub id: String,
    /// The object type, which is always `text_completion`.
    pub object: String,
    /// The Unix timestamp (in seconds) of when the chat completion was created.
    pub created: i64,
    /// The model used for the completion.
    pub model: String,
    /// A list of completion choices. Can be more than one if n is greater than 1.
    pub choices: Vec<CompletionChoice>,
    /// Usage statistics for the completion request.
    pub usage: Option<Usage>,
    /// This fingerprint represents the backend configuration that the model runs with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    /// Detections
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detections: Option<ChatDetections>,
    /// Warnings
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<OrchestratorWarning>,
}

/// Completion (legacy) choice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionChoice {
    /// The index of the choice in the list of choices.
    pub index: u32,
    /// Text generated by the model.
    pub text: String,
    /// Log probability information for the choice.
    pub logprobs: Option<CompletionLogprobs>,
    /// The reason the model stopped generating tokens.
    pub finish_reason: Option<String>,
    /// The stop string or token id that caused the completion.
    pub stop_reason: Option<String>,
    /// Prompt logprobs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_logprobs: Option<Vec<Option<HashMap<String, Logprob>>>>,
}

/// Completion logprobs.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CompletionLogprobs {
    /// Tokens generated by the model.
    pub tokens: Vec<String>,
    /// Token logprobs.
    pub token_logprobs: Vec<f32>,
    /// Top logprobs.
    pub top_logprobs: Vec<HashMap<String, f32>>,
    /// Text offsets.
    pub text_offset: Vec<u32>,
}

/// Logprob.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Logprob {
    /// The logprob of the chosen token
    pub logprob: f32,
    /// The vocab rank of the chosen token (>=1)
    pub rank: Option<i32>,
    /// The decoded chosen token index
    pub decoded_token: Option<String>,
}

/// Completion usage statistics.
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

/// Completion token details.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionTokenDetails {
    pub audio_tokens: u32,
    pub reasoning_tokens: u32,
}

/// Prompt token details.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptTokenDetails {
    pub audio_tokens: u32,
    pub cached_tokens: u32,
}

/// Stop tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StopTokens {
    Array(Vec<String>),
    String(String),
}

/// OpenAI error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiError {
    pub object: Option<String>,
    pub message: String,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
    pub param: Option<String>,
    pub code: u16,
}

/// OpenAI streaming error message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiErrorMessage {
    pub error: OpenAiError,
}

/// Guardrails chat detections.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatDetections {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<InputDetectionResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output: Vec<OutputDetectionResult>,
}

/// Guardrails chat input detections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputDetectionResult {
    pub message_index: u32,
    #[serde(default)]
    pub results: Vec<ContentAnalysisResponse>,
}

/// Guardrails chat output detections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputDetectionResult {
    pub choice_index: u32,
    #[serde(default)]
    pub results: Vec<ContentAnalysisResponse>,
}

/// Guardrails warning.
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
        let detectors = DetectorConfig {
            input: HashMap::from([("some_detector".into(), DetectorParams::new())]),
            output: HashMap::new(),
        };
        let messages = vec![Message {
            content: Some(Content::Text("Hi there!".to_string())),
            ..Default::default()
        }];
        let json_request = json!({
            "model": "test",
            "detectors": detectors,
            "messages": messages,
            "frequency_penalty": 2.0,
        });
        let request = ChatCompletionsRequest::deserialize(&json_request)?;
        let mut extra = Map::new();
        extra.insert("frequency_penalty".into(), 2.0.into());
        assert_eq!(
            request,
            ChatCompletionsRequest {
                detectors,
                stream: None,
                model: "test".into(),
                messages: messages.clone(),
                extra,
            }
        );

        // Test deserialize with no detectors
        let json_request = json!({
            "model": "test",
            "messages": messages,
        });
        let request = ChatCompletionsRequest::deserialize(&json_request)?;
        assert_eq!(
            request,
            ChatCompletionsRequest {
                detectors: DetectorConfig::default(),
                stream: None,
                model: "test".into(),
                messages: messages.clone(),
                extra: Map::new(),
            }
        );

        // Test deserialize errors
        let result = ChatCompletionsRequest::deserialize(json!({
            "detectors": DetectorConfig::default(),
            "messages": messages,
        }));
        assert!(result.is_err_and(|error| error.to_string().starts_with("missing field `model`")));

        let result = ChatCompletionsRequest::deserialize(json!({
            "model": "test",
            "detectors": DetectorConfig::default(),
            "messages": ["invalid"],
        }));
        assert!(result.is_err_and(|error| error.to_string()
            == "invalid type: string \"invalid\", expected struct Message"));

        // Test validation errors
        let request = ChatCompletionsRequest::deserialize(json!({
            "model": "",
            "detectors": DetectorConfig::default(),
            "messages": Vec::<Message>::default(),
        }))?;
        let result = request.validate();
        assert!(result.is_err_and(|error| error.to_string() == "`model` must not be empty"));

        let request = ChatCompletionsRequest::deserialize(json!({
            "model": "test",
            "detectors": DetectorConfig::default(),
            "messages": Vec::<Message>::default(),
        }))?;
        let result = request.validate();
        assert!(result.is_err_and(|error| error.to_string() == "`messages` must not be empty"));

        // Test serialize
        let request = ChatCompletionsRequest::deserialize(&json!({
            "model": "test",
            "detectors": {
                "input": {"some_detector": {}},
                "output": {},
            },
            "messages": [{"role": "user", "content": "Hi there!"}],
            "frequency_penalty": 2.0,
        }))?;
        let serialized_request = serde_json::to_value(request)?;
        // should include stream: false and exclude detectors
        assert_eq!(
            serialized_request,
            json!({
                "model": "test",
                "messages": [{"role": "user", "content": "Hi there!"}],
                "frequency_penalty": 2.0,
            })
        );

        Ok(())
    }
}
