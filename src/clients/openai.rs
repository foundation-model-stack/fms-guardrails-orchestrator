use std::collections::HashMap;

use async_trait::async_trait;
use hyper::StatusCode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{Client, Error, HttpClient};
use crate::health::HealthCheckResult;

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct OpenAiClient {
    client: HttpClient,
}

#[cfg_attr(test, faux::methods)]
impl OpenAiClient {
    pub fn new(client: HttpClient) -> Self {
        Self { client }
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

    pub async fn completions(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, Error> {
        let url = self.client.base_url().join("/v1/completions").unwrap();
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
        self.client.health().await
    }
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StopTokens {
    Array(Vec<String>),
    String(String),
}

// Chat completions API types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    /// ID of the model to use.
    pub model: String,
    /// A list of messages comprising the conversation so far.
    pub messages: Vec<Message>,
    #[serde(default)]
    pub frequency_penalty: Option<f32>,
    /// Modify the likelihood of specified tokens appearing in the completion.
    #[serde(default)]
    pub logit_bias: Option<HashMap<String, f32>>,
    /// Whether to return log probabilities of the output tokens or not.
    /// If true, returns the log probabilities of each output token returned in the content of message.
    #[serde(default)]
    pub logprobs: Option<bool>,
    /// An integer between 0 and 20 specifying the number of most likely tokens to return
    /// at each token position, each with an associated log probability.
    /// logprobs must be set to true if this parameter is used.
    #[serde(default)]
    pub top_logprobs: Option<u32>,
    /// The maximum number of tokens that can be generated in the chat completion.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// How many chat completion choices to generate for each input message.
    #[serde(default)]
    pub n: Option<u32>,
    /// Positive values penalize new tokens based on whether they appear in the text so far,
    /// increasing the model's likelihood to talk about new topics.
    #[serde(default)]
    pub presence_penalty: Option<f32>,
    //#[serde(default)]
    //pub response_format: Option<ResponseFormat>,
    /// If specified, our system will make a best effort to sample deterministically,
    /// such that repeated requests with the same seed and parameters should return the same result.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Up to 4 sequences where the API will stop generating further tokens.
    #[serde(default)]
    pub stop: Option<StopTokens>,
    /// If set, partial message deltas will be sent, like in ChatGPT.
    /// Tokens will be sent as data-only server-sent events as they become available,
    /// with the stream terminated by a data: [DONE] message.
    #[serde(default)]
    pub stream: Option<bool>,
    /// What sampling temperature to use, between 0 and 2.
    /// Higher values like 0.8 will make the output more random,
    /// while lower values like 0.2 will make it more focused and deterministic.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// An alternative to sampling with temperature, called nucleus sampling,
    /// where the model considers the results of the tokens with top_p probability mass.
    /// So 0.1 means only the tokens comprising the top 10% probability mass are considered.
    #[serde(default)]
    pub top_p: Option<f32>,

    // Additional vllm params
    #[serde(default)]
    pub best_of: Option<usize>,
    #[serde(default)]
    pub use_beam_search: Option<bool>,
    #[serde(default)]
    pub top_k: Option<isize>,
    #[serde(default)]
    pub min_p: Option<f32>,
    #[serde(default)]
    pub repetition_penalty: Option<f32>,
    #[serde(default)]
    pub length_penalty: Option<f32>,
    #[serde(default)]
    pub early_stopping: Option<bool>,
    #[serde(default)]
    pub ignore_eos: Option<bool>,
    #[serde(default)]
    pub min_tokens: Option<u32>,
    #[serde(default)]
    pub stop_token_ids: Option<Vec<usize>>,
    #[serde(default)]
    pub skip_special_tokens: Option<bool>,
    #[serde(default)]
    pub spaces_between_special_tokens: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn new(role: &str, content: &str, name: Option<&str>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            name: name.map(|s| s.into()),
        }
    }
}

/// Represents a chat completion response returned by model, based on the provided input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    /// A unique identifier for the chat completion.
    pub id: String,
    /// The object type, which is always `chat.completion`.
    pub object: String,
    /// The Unix timestamp (in seconds) of when the chat completion was created.
    pub created: i64,
    /// The model used for the chat completion.
    pub model: String,
    /// This fingerprint represents the backend configuration that the model runs with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    /// A list of chat completion choices. Can be more than one if n is greater than 1.
    pub choices: Vec<ChatCompletionChoice>,
    /// Usage statistics for the completion request.
    pub usage: Usage,
}

/// A chat completion choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChoice {
    /// The index of the choice in the list of choices.
    pub index: usize,
    /// A chat completion message generated by the model.
    pub message: ChatCompletionMessage,
    /// Log probability information for the choice.
    pub logprobs: Option<ChatCompletionLogprobs>,
    /// The reason the model stopped generating tokens.
    pub finish_reason: String,
}

/// A chat completion message generated by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionMessage {
    /// The contents of the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The role of the author of this message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionLogprobs {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ChatCompletionLogprob>,
}

/// Log probability information for a choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionLogprob {
    /// The token.
    pub token: String,
    /// The log probability of this token.
    pub logprob: f32,
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
    /// The object type, which is always `chat.completion.chunk`.
    pub object: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunkChoice {
    /// A chat completion delta generated by streamed model responses.
    pub delta: ChatCompletionMessage,
    /// The index of the choice in the list of choices.
    pub index: u32,
    /// Log probability information for the choice.
    pub logprobs: Option<ChatCompletionLogprobs>,
    /// The reason the model stopped generating tokens.
    pub finish_reason: Option<String>,
}

// Completions API types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// ID of the model to use.
    pub model: String,
    /// The prompt to generate completions for.
    /// NOTE: Only supporting a single prompt for now. OpenAI supports a single string,
    /// array of strings, array of tokens, or an array of token arrays.
    pub prompt: String,
    /// Generates best_of completions server-side and returns the "best" (the one with the highest log probability per token).
    /// Results cannot be streamed. When used with n, best_of controls the number of candidate completions and n specifies
    /// how many to return – best_of must be greater than n.
    #[serde(default)]
    pub best_of: Option<u32>,
    /// Echo back the prompt in addition to the completion.
    #[serde(default)]
    pub echo: Option<bool>,
    /// Positive values penalize new tokens based on their existing frequency in the text so far,
    /// decreasing the model's likelihood to repeat the same line verbatim.
    #[serde(default)]
    pub frequency_penalty: Option<f32>,
    /// Modify the likelihood of specified tokens appearing in the completion.
    #[serde(default)]
    pub logit_bias: Option<HashMap<String, f32>>,
    /// Include the log probabilities on the logprobs most likely output tokens, as well the chosen tokens.
    #[serde(default)]
    pub logprobs: Option<u32>,
    /// The maximum number of tokens that can be generated in the completion.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// How many completions to generate for each prompt.
    #[serde(default)]
    pub n: Option<u32>,
    /// Positive values penalize new tokens based on whether they appear in the text so far,
    /// increasing the model's likelihood to talk about new topics.
    #[serde(default)]
    pub presence_penalty: Option<f32>,
    /// If specified, our system will make a best effort to sample deterministically,
    /// such that repeated requests with the same seed and parameters should return the same result.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Up to 4 sequences where the API will stop generating further tokens.
    /// The returned text will not contain the stop sequence.
    #[serde(default)]
    pub stop: Option<StopTokens>,
    /// Whether to stream back partial progress.
    /// If set, tokens will be sent as data-only server-sent events as they become available,
    /// with the stream terminated by a data: [DONE] message.
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    /// The suffix that comes after a completion of inserted text.
    pub suffix: Option<String>,
    /// What sampling temperature to use, between 0 and 2.
    /// Higher values like 0.8 will make the output more random,
    /// while lower values like 0.2 will make it more focused and deterministic.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// An alternative to sampling with temperature, called nucleus sampling,
    /// where the model considers the results of the tokens with top_p probability mass.
    /// So 0.1 means only the tokens comprising the top 10% probability mass are considered.
    #[serde(default)]
    pub top_p: Option<f32>,

    // Additional vllm params
    #[serde(default)]
    pub use_beam_search: Option<bool>,
    #[serde(default)]
    pub top_k: Option<isize>,
    #[serde(default)]
    pub min_p: Option<f32>,
    #[serde(default)]
    pub repetition_penalty: Option<f32>,
    #[serde(default)]
    pub length_penalty: Option<f32>,
    #[serde(default)]
    pub early_stopping: Option<bool>,
    #[serde(default)]
    pub stop_token_ids: Option<Vec<usize>>,
    #[serde(default)]
    pub ignore_eos: Option<bool>,
    #[serde(default)]
    pub min_tokens: Option<u32>,
    #[serde(default)]
    pub skip_special_tokens: Option<bool>,
    #[serde(default)]
    pub spaces_between_special_tokens: Option<bool>,
}

// #[derive(Debug, Clone, Serialize, Deserialize)]
// #[serde(untagged)]
// pub enum Prompt {
//     Array(Vec<String>),
//     String(String),
// }

/// Represents a completion response from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// A unique identifier for the completion.
    pub id: String,
    /// The object type, which is always `text_completion`.
    pub object: String,
    /// The Unix timestamp (in seconds) of when the completion was created.
    pub created: i64,
    /// The model used for the completion.
    pub model: String,
    /// This fingerprint represents the backend configuration that the model runs with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    /// A list of completion choices. Can be more than one if n is greater than 1.
    pub choices: Vec<CompletionChoice>,
    /// Usage statistics for the completion request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

/// A completion choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionChoice {
    /// The index of the choice in the list of choices.
    pub index: u32,
    /// A chat completion message generated by the model.
    pub text: Option<String>,
    /// Log probability information for the choice.
    pub logprobs: Option<CompletionLogprobs>,
    /// The reason the model stopped generating tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// Log probability information for a choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionLogprobs {
    pub text_offset: Vec<u32>,
    pub token_logprobs: Vec<f32>,
    pub tokens: Vec<String>,
    pub top_logprobs: Option<Vec<IndexMap<String, f32>>>,
}
