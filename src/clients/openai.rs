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

use async_trait::async_trait;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

use super::{create_http_client, Client, Error, HttpClient};
use crate::{config::ServiceConfig, health::HealthCheckResult};

const DEFAULT_PORT: u16 = 8080;

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct OpenAiClient {
    client: HttpClient,
    health_client: Option<HttpClient>,
}

#[cfg_attr(test, faux::methods)]
impl OpenAiClient {
    pub async fn new(config: &ServiceConfig, health_config: Option<&ServiceConfig>) -> Self {
        let client = create_http_client(DEFAULT_PORT, config).await;
        let health_client = if let Some(health_config) = health_config {
            Some(create_http_client(DEFAULT_PORT, health_config).await)
        } else {
            None
        };
        Self {
            client,
            health_client,
        }
    }

    pub async fn chat_completions(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, Error> {
        let url = self.client.base_url().join("/v1/chat/completions").unwrap();
        let response = self.client.post(url).json(&request).send().await?;
        match response.status() {
            StatusCode::OK => Ok(response.json().await?),
            _ => Err(Error::Http {
                code: response.status(),
                message: "".into(), // TODO
            }),
        }
    }
}

#[cfg_attr(test, faux::methods)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    /// A list of messages comprising the conversation so far.
    pub messages: Vec<Message>,
    /// ID of the model to use.
    pub model: String,
    /// Whether or not to store the output of this chat completion request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    /// Developer-defined tags and values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    /// Modify the likelihood of specified tokens appearing in the completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<HashMap<String, f32>>,
    /// Whether to return log probabilities of the output tokens or not.
    /// If true, returns the log probabilities of each output token returned in the content of message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    /// An integer between 0 and 20 specifying the number of most likely tokens to return
    /// at each token position, each with an associated log probability.
    /// logprobs must be set to true if this parameter is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u32>,
    /// The maximum number of tokens that can be generated in the chat completion. (DEPRECATED)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// An upper bound for the number of tokens that can be generated for a completion, including visible output tokens and reasoning tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    /// How many chat completion choices to generate for each input message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// Positive values penalize new tokens based on whether they appear in the text so far,
    /// increasing the model's likelihood to talk about new topics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    /// An object specifying the format that the model must output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    /// If specified, our system will make a best effort to sample deterministically,
    /// such that repeated requests with the same seed and parameters should return the same result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Specifies the latency tier to use for processing the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    /// Up to 4 sequences where the API will stop generating further tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<StopTokens>,
    /// If set, partial message deltas will be sent, like in ChatGPT.
    /// Tokens will be sent as data-only server-sent events as they become available,
    /// with the stream terminated by a data: [DONE] message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Options for streaming response. Only set this when you set stream: true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
    /// What sampling temperature to use, between 0 and 2.
    /// Higher values like 0.8 will make the output more random,
    /// while lower values like 0.2 will make it more focused and deterministic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// An alternative to sampling with temperature, called nucleus sampling,
    /// where the model considers the results of the tokens with top_p probability mass.
    /// So 0.1 means only the tokens comprising the top 10% probability mass are considered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// A list of tools the model may call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Tool>,
    /// Controls which (if any) tool is called by the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    /// Whether to enable parallel function calling during tool use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// A unique identifier representing your end-user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    // Additional vllm params
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_of: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_beam_search: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<isize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repetition_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub early_stopping: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_eos: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_token_ids: Option<Vec<usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_special_tokens: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spaces_between_special_tokens: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFormat {
    /// The type of response format being defined.
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<JsonSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSchema {
    /// The name of the response format.
    pub name: String,
    /// A description of what the response format is for, used by the model to determine how to respond in the format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The schema for the response format, described as a JSON Schema object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<JsonSchemaObject>,
    /// Whether to enable strict schema adherence when generating the output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<JsonSchema>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSchemaObject {
    pub id: String,
    pub schema: String,
    pub title: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub r#type: String,
    pub properties: Option<HashMap<String, serde_json::Value>>,
    pub required: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// The role of the messages author.
    pub role: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    /// The text contents of the message.
    String(String),
    /// Array of content parts.
    Array(Vec<ContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPart {
    /// The type of the content part.
    #[serde(rename = "type")]
    pub r#type: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    /// Either a URL of the image or the base64 encoded image data.
    pub url: String,
    /// Specifies the detail level of the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// The ID of the tool call.
    pub id: String,
    /// The type of the tool.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The function that the model called.
    pub function: Function,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    /// The name of the function to call.
    pub name: String,
    /// The arguments to call the function with, as generated by the model in JSON format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<String>>,
}

/// Represents a chat completion response returned by model, based on the provided input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    /// A unique identifier for the chat completion.
    pub id: String,
    /// A list of chat completion choices. Can be more than one if n is greater than 1.
    pub choices: Vec<ChatCompletionChoice>,
    /// The Unix timestamp (in seconds) of when the chat completion was created.
    pub created: i64,
    /// The model used for the chat completion.
    pub model: String,
    /// The service tier used for processing the request.
    /// This field is only included if the `service_tier` parameter is specified in the request.
    pub service_tier: Option<String>,
    /// This fingerprint represents the backend configuration that the model runs with.
    #[serde(default)]
    pub system_fingerprint: String,
    /// The object type, which is always `chat.completion`.
    pub object: String,
    /// Usage statistics for the completion request.
    pub usage: Usage,
}

/// A chat completion choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChoice {
    /// The reason the model stopped generating tokens.
    pub finish_reason: String,
    /// The index of the choice in the list of choices.
    pub index: usize,
    /// A chat completion message generated by the model.
    pub message: ChatCompletionMessage,
    /// Log probability information for the choice.
    pub logprobs: Option<ChatCompletionLogprobs>,
}

/// A chat completion message generated by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionMessage {
    /// The contents of the message.
    pub content: Option<String>,
    /// The refusal message generated by the model.
    pub refusal: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// The role of the author of this message.
    pub role: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionLogprobs {
    /// A list of message content tokens with log probability information.
    pub content: Option<Vec<ChatCompletionLogprob>>,
    /// A list of message refusal tokens with log probability information.
    pub refusal: Option<Vec<ChatCompletionLogprob>>,
}

/// Log probability information for a choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionLogprob {
    /// The token.
    pub token: String,
    /// The log probability of this token.
    pub logprob: f32,
    pub bytes: Option<Vec<u8>>,
    /// List of the most likely tokens and their log probability, at this token position.
    pub top_logprobs: Option<Vec<ChatCompletionTopLogprob>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionTopLogprob {
    /// The token.
    pub token: String,
    /// The log probability of this token.
    pub logprob: f32,
}

/// Represents a streamed chunk of a chat completion response returned by model, based on the provided input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunk {
    /// A unique identifier for the chat completion. Each chunk has the same ID.
    pub id: String,
    /// A list of chat completion choices.
    pub choices: Vec<ChatCompletionChunkChoice>,
    /// The Unix timestamp (in seconds) of when the chat completion was created. Each chunk has the same timestamp.
    pub created: i64,
    /// The model to generate the completion.
    pub model: String,
    /// The service tier used for processing the request.
    /// This field is only included if the service_tier parameter is specified in the request.
    pub service_tier: Option<String>,
    /// This fingerprint represents the backend configuration that the model runs with.
    pub system_fingerprint: String,
    /// The object type, which is always `chat.completion.chunk`.
    pub object: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunkChoice {
    /// A chat completion delta generated by streamed model responses.
    pub delta: ChatCompletionMessage,
    /// Log probability information for the choice.
    pub logprobs: Option<ChatCompletionLogprobs>,
    /// The reason the model stopped generating tokens.
    pub finish_reason: Option<String>,
    /// The index of the choice in the list of choices.
    pub index: u32,
}

/// Usage statistics for a completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Number of tokens in the generated completion.
    pub completion_tokens: u32,
    /// Number of tokens in the prompt.
    pub prompt_tokens: u32,
    /// Total number of tokens used in the request (prompt + completion).
    pub total_tokens: u32,
    /// Breakdown of tokens used in a completion.
    pub completion_token_details: CompletionTokenDetails,
    /// Breakdown of tokens used in the prompt.
    pub prompt_token_details: PromptTokenDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionTokenDetails {
    pub audio_tokens: u32,
    pub reasoning_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
