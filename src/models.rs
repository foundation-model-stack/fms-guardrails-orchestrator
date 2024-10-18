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

#![allow(unused_qualifications)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    clients::{
        self,
        detector::{ContentAnalysisResponse, ContextType},
        openai::Content,
    },
    health::HealthCheckCache,
    pb,
};

#[derive(Clone, Debug, Serialize)]
pub struct InfoResponse {
    pub services: HealthCheckCache,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InfoParams {
    /// Whether to probe the client services' health checks or just return the latest health status.
    #[serde(default)]
    pub probe: bool,
}

/// Parameters relevant to each detector
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DetectorParams(HashMap<String, serde_json::Value>);

impl DetectorParams {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Threshold to filter detector results by score.
    pub fn threshold(&self) -> Option<f64> {
        self.0.get("threshold").and_then(|v| v.as_f64())
    }
}

impl std::ops::Deref for DetectorParams {
    type Target = HashMap<String, serde_json::Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for DetectorParams {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// User request to orchestrator
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuardrailsHttpRequest {
    /// Text generation model ID
    pub model_id: String,

    /// User prompt/input text to a text generation model
    pub inputs: String,

    /// Configuration of guardrails models for either or both input to a text generation model
    /// (e.g. user prompt) and output of a text generation model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrail_config: Option<GuardrailsConfig>,

    /// Parameters for text generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("`{0}` is required")]
    Required(String),
    #[error("{0}")]
    Invalid(String),
}

impl GuardrailsHttpRequest {
    /// Upfront validation of user request
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Validate required parameters
        if self.model_id.is_empty() {
            return Err(ValidationError::Required("model_id".into()));
        }
        if self.inputs.is_empty() {
            return Err(ValidationError::Required("inputs".into()));
        }

        let guardrail_config = self.guardrail_config.as_ref();

        // Validate masks
        // Because the masks ranges are [start, end), while applying masks
        // will not require indexing to include the last index (i.e. len of inputs),
        // the last index is still a legitimate 'end' to provide on a mask here.
        let input_range = 0..=self.inputs.len();
        let input_masks = guardrail_config
            .and_then(|config| config.input.as_ref().and_then(|input| input.masks.as_ref()));
        if let Some(input_masks) = input_masks {
            if !input_masks.iter().all(|(start, end)| {
                input_range.contains(start) && input_range.contains(end) && start < end
            }) {
                return Err(ValidationError::Invalid("invalid masks".into()));
            }
        }

        // Validate detector params
        if let Some(config) = guardrail_config {
            if let Some(input_detectors) = config.input_detectors() {
                validate_detector_params(input_detectors)?;
            }
            if let Some(output_detectors) = config.output_detectors() {
                validate_detector_params(output_detectors)?;
            }
        }

        Ok(())
    }
}

/// Configuration of guardrails models for either or both input to a text generation model
/// (e.g. user prompt) and output of a text generation model
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardrailsConfig {
    /// Configuration for detection on input to a text generation model (e.g. user prompt)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<GuardrailsConfigInput>,

    /// Configuration for detection on output of a text generation model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<GuardrailsConfigOutput>,
}

impl GuardrailsConfig {
    pub fn input_masks(&self) -> Option<&[(usize, usize)]> {
        self.input.as_ref().and_then(|input| input.masks.as_deref())
    }

    pub fn input_detectors(&self) -> Option<&HashMap<String, DetectorParams>> {
        self.input.as_ref().map(|input| &input.models)
    }

    pub fn output_detectors(&self) -> Option<&HashMap<String, DetectorParams>> {
        self.output.as_ref().map(|output| &output.models)
    }
}

/// Configuration for detection on input to a text generation model (e.g. user prompt)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardrailsConfigInput {
    /// Map of model name to model specific parameters
    pub models: HashMap<String, DetectorParams>,
    /// Vector of spans are in the form of (span_start, span_end) corresponding
    /// to spans of input text on which to run input detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub masks: Option<Vec<(usize, usize)>>,
}

/// Configuration for detection on output of a text generation model
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardrailsConfigOutput {
    /// Map of model name to model specific parameters
    pub models: HashMap<String, DetectorParams>,
}

/// Parameters for text generation, ref. <https://github.com/IBM/text-generation-inference/blob/main/proto/generation.proto>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardrailsTextGenerationParameters {
    // Leave most validation of parameters to downstream text generation servers
    /// Maximum number of new tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_new_tokens: Option<u32>,

    /// Minimum number of new tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_new_tokens: Option<u32>,

    /// Truncate to this many input tokens for generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncate_input_tokens: Option<u32>,

    /// The high level decoding strategy for picking
    /// tokens during text generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoding_method: Option<String>,

    /// Number of highest probability vocabulary tokens to keep for top-k-filtering.
    /// Only applies for sampling mode. When decoding_strategy is set to sample,
    /// only the top_k most likely tokens are considered as candidates for the next generated token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,

    /// Similar to top_k except the candidates to generate the next token are the
    /// most likely tokens with probabilities that add up to at least top_p.
    /// Also known as nucleus sampling. A value of 1.0 is equivalent to disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    /// Local typicality measures how similar the conditional probability of
    /// predicting a target token next is to the expected conditional
    /// probability of predicting a random token next, given the partial text
    /// already generated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typical_p: Option<f64>,

    /// A value used to modify the next-token probabilities in sampling mode.
    /// Values less than 1.0 sharpen the probability distribution, resulting in
    /// "less random" output. Values greater than 1.0 flatten the probability distribution,
    /// resulting in "more random" output. A value of 1.0 has no effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Represents the penalty for penalizing tokens that have already been generated
    /// or belong to the context. The value 1.0 means that there is no penalty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repetition_penalty: Option<f64>,

    /// Time limit in milliseconds for text generation to complete
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_time: Option<f64>,

    /// Parameters to exponentially increase the likelihood of the text generation
    /// terminating once a specified number of tokens have been generated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exponential_decay_length_penalty: Option<ExponentialDecayLengthPenalty>,

    /// One or more strings which will cause the text generation to stop if/when
    /// they are produced as part of the output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,

    /// Random seed used for text generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,

    /// Whether or not to include input text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preserve_input_text: Option<bool>,

    /// Whether or not to include input text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<bool>,

    /// Whether or not to include list of individual generated tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_tokens: Option<bool>,

    /// Whether or not to include logprob for each returned token
    /// Applicable only if generated_tokens == true and/or input_tokens == true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_logprobs: Option<bool>,

    /// Whether or not to include rank of each returned token
    /// Applicable only if generated_tokens == true and/or input_tokens == true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_ranks: Option<bool>,

    /// Whether or not to include stop sequence
    /// If not specified, default behavior depends on server setting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_stop_sequence: Option<bool>,
}

/// Parameters to exponentially increase the likelihood of the text generation
/// terminating once a specified number of tokens have been generated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExponentialDecayLengthPenalty {
    /// Start the decay after this number of tokens have been generated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_index: Option<u32>,

    /// Factor of exponential decay
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decay_factor: Option<f64>,
}

/// Classification result on text produced by a text generation model, containing
/// information from the original text generation output as well as the result of
/// classification on the generated text.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassifiedGeneratedTextResult {
    /// Generated text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_text: Option<String>,

    /// Classification results for input to text generation model and/or
    /// output from the text generation model
    pub token_classification_results: TextGenTokenClassificationResults,

    /// Why text generation stopped
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    /// Length of sequence of generated tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_token_count: Option<u32>,

    /// Random seed used for text generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,

    /// Length of input
    pub input_token_count: u32,

    /// Vector of warnings on input detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<InputWarning>>,

    /// Individual generated tokens and associated details, if requested
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    /// Input tokens and associated details, if requested
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,
}

/// The request format expected in the /api/v2/text/detection/content endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextContentDetectionHttpRequest {
    /// The content to run detectors on
    pub content: String,

    /// The map of detectors to be used, along with their respective parameters, e.g. thresholds.
    pub detectors: HashMap<String, DetectorParams>,
}

impl TextContentDetectionHttpRequest {
    /// Upfront validation of user request
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Validate required parameters
        if self.content.is_empty() {
            return Err(ValidationError::Required("content".into()));
        }
        if self.detectors.is_empty() {
            return Err(ValidationError::Required("detectors".into()));
        }

        // Validate detector params
        validate_detector_params(&self.detectors)?;

        Ok(())
    }
}

/// The response format of the /api/v2/text/detection/content endpoint
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextContentDetectionResult {
    /// Detection results
    pub detections: Vec<ContentAnalysisResponse>,
}
/// Streaming classification result on text produced by a text generation model, containing
/// information from the original text generation output as well as the result of
/// classification on the generated text. Also indicates where in stream is processed.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassifiedGeneratedTextStreamResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_text: Option<String>,

    /// Classification results for input to text generation model and/or
    /// output from the text generation model
    pub token_classification_results: TextGenTokenClassificationResults,

    /// Why text generation stopped
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    /// Length of sequence of generated tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_token_count: Option<u32>,

    /// Random seed used for text generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,

    /// Length of input
    pub input_token_count: u32,

    /// Vector of warnings on input detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<InputWarning>>,

    /// Individual generated tokens and associated details, if requested
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    /// Input tokens and associated details, if requested
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,

    /// Result index up to which text is processed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_index: Option<u32>,

    /// Result start index for processed text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_index: Option<u32>,
}

/// Results of classification on input to a text generation model (e.g. user prompt)
/// or output of a text generation model
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextGenTokenClassificationResults {
    /// Classification results on input to a text generation model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<TokenClassificationResult>>,

    /// Classification results on output from a text generation model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<TokenClassificationResult>>,
}

/// Single token classification result
/// NOTE: This is meant to align with the HuggingFace token classification task:
/// <https://huggingface.co/docs/transformers/tasks/token_classification#inference>
/// The field `word` does not necessarily correspond to a single "word",
/// and `entity` may not always be applicable beyond "entity" in the NER
/// (named entity recognition) sense
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenClassificationResult {
    /// Beginning/start offset of token
    pub start: u32,

    /// End offset of token
    pub end: u32,

    /// Text referenced by token
    pub word: String,

    /// Predicted relevant class name for the token
    pub entity: String,

    /// Aggregate label, if applicable
    pub entity_group: String,

    /// Confidence-like score of this classification prediction in [0, 1]
    pub score: f64,

    /// Length of tokens in the text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u32>,
}

/// Enumeration of reasons why text generation stopped
/// Since this enum's variants do not hold data, we can easily define them as `#[repr(C)]`
/// which helps with FFI.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FinishReason {
    #[serde(rename = "NOT_FINISHED")]
    NotFinished,
    #[serde(rename = "MAX_TOKENS")]
    MaxTokens,
    #[serde(rename = "EOS_TOKEN")]
    EosToken,
    #[serde(rename = "CANCELLED")]
    Cancelled,
    #[serde(rename = "TIME_LIMIT")]
    TimeLimit,
    #[serde(rename = "STOP_SEQUENCE")]
    StopSequence,
    #[serde(rename = "TOKEN_LIMIT")]
    TokenLimit,
    #[serde(rename = "ERROR")]
    Error,
}

/// Warning reason and message on input detection
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputWarning {
    /// Warning reason
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<InputWarningReason>,

    /// Warning message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Enumeration of warning reasons on input detection
/// Since this enum's variants do not hold data, we can easily define them as `#[repr(C)]`
/// which helps with FFI.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InputWarningReason {
    /// Unsuitable text detected on input
    #[serde(rename = "UNSUITABLE_INPUT")]
    UnsuitableInput,
}

/// Generated token information
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedToken {
    /// Token text
    pub text: String,

    /// Logprob (log of normalized probability)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprob: Option<f64>,

    /// One-based rank relative to other tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
}

/// Result of a text generation model
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedTextResult {
    /// Generated text
    pub generated_text: String,

    /// Length of sequence of generated tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_tokens: Option<i32>,

    /// Why text generation stopped
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    /// Length of input
    pub input_token_count: u32,

    /// Random seed used for text generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,

    /// Individual generated tokens and associated details, if requested
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    /// Input tokens and associated details, if requested
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,
}

/// Details on the streaming result of a text generation model
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenStreamDetails {
    /// Why text generation stopped
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    /// Length of sequence of generated tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_tokens: Option<u32>,

    /// Random seed used for text generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,

    /// Length of input
    pub input_token_count: u32,
}

/// Streaming result of a text generation model
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedTextStreamResult {
    /// Generated text
    pub generated_text: String,

    /// Individual generated tokens and associated details, if requested
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    /// Details on the streaming result of a text generation model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<TokenStreamDetails>,

    /// Streaming result of a text generation model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,
}

impl From<ExponentialDecayLengthPenalty> for pb::fmaas::decoding_parameters::LengthPenalty {
    fn from(value: ExponentialDecayLengthPenalty) -> Self {
        Self {
            start_index: value.start_index.unwrap_or_default(),
            decay_factor: value.decay_factor.unwrap_or_default() as f32,
        }
    }
}

impl From<GuardrailsTextGenerationParameters> for pb::fmaas::Parameters {
    // NOTE: types should really be consistent between APIs
    fn from(value: GuardrailsTextGenerationParameters) -> Self {
        let decoding_method = value.decoding_method.unwrap_or("GREEDY".to_string());
        let method = pb::fmaas::DecodingMethod::from_str_name(&decoding_method).unwrap_or_default();
        let sampling = pb::fmaas::SamplingParameters {
            temperature: value.temperature.unwrap_or_default() as f32,
            top_k: value.top_k.unwrap_or_default(),
            top_p: value.top_p.unwrap_or_default() as f32,
            typical_p: value.typical_p.unwrap_or_default() as f32,
            seed: value.seed.map(|v| v as u64),
        };
        let stopping = pb::fmaas::StoppingCriteria {
            max_new_tokens: value.max_new_tokens.unwrap_or_default(),
            min_new_tokens: value.min_new_tokens.unwrap_or_default(),
            time_limit_millis: value.max_time.unwrap_or_default() as u32,
            stop_sequences: value.stop_sequences.unwrap_or_default(),
            include_stop_sequence: value.include_stop_sequence,
        };
        let response = pb::fmaas::ResponseOptions {
            input_text: value.preserve_input_text.unwrap_or_default(),
            generated_tokens: value.generated_tokens.unwrap_or_default(),
            input_tokens: value.input_tokens.unwrap_or_default(),
            token_logprobs: value.token_logprobs.unwrap_or_default(),
            token_ranks: value.token_ranks.unwrap_or_default(),
            top_n_tokens: 0, // missing?
        };
        let decoding = pb::fmaas::DecodingParameters {
            repetition_penalty: value.repetition_penalty.unwrap_or_default() as f32,
            length_penalty: value.exponential_decay_length_penalty.map(Into::into),
        };
        let truncate_input_tokens = value.truncate_input_tokens.unwrap_or_default();
        Self {
            method: method as i32,
            sampling: Some(sampling),
            stopping: Some(stopping),
            response: Some(response),
            decoding: Some(decoding),
            truncate_input_tokens,
            beam: None, // missing?
        }
    }
}

impl From<pb::fmaas::StopReason> for FinishReason {
    fn from(value: pb::fmaas::StopReason) -> Self {
        use pb::fmaas::StopReason::*;
        match value {
            NotFinished => FinishReason::NotFinished,
            MaxTokens => FinishReason::MaxTokens,
            EosToken => FinishReason::EosToken,
            Cancelled => FinishReason::Cancelled,
            TimeLimit => FinishReason::TimeLimit,
            StopSequence => FinishReason::StopSequence,
            TokenLimit => FinishReason::TokenLimit,
            Error => FinishReason::Error,
        }
    }
}

impl From<pb::fmaas::TokenInfo> for GeneratedToken {
    fn from(value: pb::fmaas::TokenInfo) -> Self {
        Self {
            text: value.text,
            logprob: Some(value.logprob as f64),
            rank: Some(value.rank),
        }
    }
}

impl From<pb::caikit_data_model::nlp::GeneratedToken> for GeneratedToken {
    fn from(value: pb::caikit_data_model::nlp::GeneratedToken) -> Self {
        Self {
            text: value.text,
            logprob: Some(value.logprob),
            rank: Some(value.rank as u32),
        }
    }
}

impl From<pb::caikit_data_model::nlp::FinishReason> for FinishReason {
    fn from(value: pb::caikit_data_model::nlp::FinishReason) -> Self {
        use pb::caikit_data_model::nlp::FinishReason::*;
        match value {
            NotFinished => FinishReason::NotFinished,
            MaxTokens => FinishReason::MaxTokens,
            EosToken => FinishReason::EosToken,
            Cancelled => FinishReason::Cancelled,
            TimeLimit => FinishReason::TimeLimit,
            StopSequence => FinishReason::StopSequence,
            TokenLimit => FinishReason::TokenLimit,
            Error => FinishReason::Error,
        }
    }
}

impl From<ExponentialDecayLengthPenalty>
    for pb::caikit_data_model::caikit_nlp::ExponentialDecayLengthPenalty
{
    fn from(value: ExponentialDecayLengthPenalty) -> Self {
        Self {
            start_index: value.start_index.map(|v| v as i64).unwrap_or_default(),
            decay_factor: value.decay_factor.unwrap_or_default(),
        }
    }
}

impl From<pb::fmaas::GenerationResponse> for ClassifiedGeneratedTextStreamResult {
    fn from(value: pb::fmaas::GenerationResponse) -> Self {
        Self {
            generated_text: Some(value.text.clone()),
            finish_reason: Some(value.stop_reason().into()),
            generated_token_count: Some(value.generated_token_count),
            seed: Some(value.seed as u32),
            input_token_count: value.input_token_count,
            warnings: None,
            tokens: if value.tokens.is_empty() {
                None
            } else {
                Some(value.tokens.into_iter().map(Into::into).collect())
            },
            input_tokens: if value.input_tokens.is_empty() {
                None
            } else {
                Some(value.input_tokens.into_iter().map(Into::into).collect())
            },
            token_classification_results: TextGenTokenClassificationResults {
                input: None,
                output: None,
            },
            processed_index: None,
            start_index: Some(0),
        }
    }
}

impl From<pb::fmaas::BatchedGenerationResponse> for ClassifiedGeneratedTextResult {
    fn from(mut value: pb::fmaas::BatchedGenerationResponse) -> Self {
        let value = value.responses.swap_remove(0);
        Self {
            generated_text: Some(value.text.clone()),
            finish_reason: Some(value.stop_reason().into()),
            generated_token_count: Some(value.generated_token_count),
            seed: Some(value.seed as u32),
            input_token_count: value.input_token_count,
            warnings: None,
            tokens: if value.tokens.is_empty() {
                None
            } else {
                Some(value.tokens.into_iter().map(Into::into).collect())
            },
            input_tokens: if value.input_tokens.is_empty() {
                None
            } else {
                Some(value.input_tokens.into_iter().map(Into::into).collect())
            },
            token_classification_results: TextGenTokenClassificationResults {
                input: None,
                output: None,
            },
        }
    }
}

impl From<pb::caikit_data_model::nlp::GeneratedTextStreamResult>
    for ClassifiedGeneratedTextStreamResult
{
    fn from(value: pb::caikit_data_model::nlp::GeneratedTextStreamResult) -> Self {
        let details = value.details.as_ref();
        Self {
            generated_text: Some(value.generated_text.clone()),
            finish_reason: details.map(|v| v.finish_reason().into()),
            generated_token_count: details.map(|v| v.generated_tokens),
            seed: details.map(|v| v.seed as u32),
            input_token_count: details
                .map(|v| v.input_token_count as u32)
                .unwrap_or_default(), // TODO
            warnings: None,
            tokens: if value.tokens.is_empty() {
                None
            } else {
                Some(value.tokens.into_iter().map(Into::into).collect())
            },
            input_tokens: if value.input_tokens.is_empty() {
                None
            } else {
                Some(value.input_tokens.into_iter().map(Into::into).collect())
            },
            token_classification_results: TextGenTokenClassificationResults {
                input: None,
                output: None,
            },
            processed_index: None,
            start_index: None,
        }
    }
}

impl From<pb::caikit_data_model::nlp::GeneratedTextResult> for ClassifiedGeneratedTextResult {
    fn from(value: pb::caikit_data_model::nlp::GeneratedTextResult) -> Self {
        Self {
            generated_text: Some(value.generated_text.clone()),
            finish_reason: Some(value.finish_reason().into()),
            generated_token_count: Some(value.generated_tokens as u32),
            seed: Some(value.seed as u32),
            input_token_count: value.input_token_count as u32,
            warnings: None,
            tokens: if value.tokens.is_empty() {
                None
            } else {
                Some(value.tokens.into_iter().map(Into::into).collect())
            },
            input_tokens: if value.input_tokens.is_empty() {
                None
            } else {
                Some(value.input_tokens.into_iter().map(Into::into).collect())
            },
            token_classification_results: TextGenTokenClassificationResults {
                input: None,
                output: None,
            },
        }
    }
}

/// The request format expected in the /api/v2/text/generation-detection endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenerationWithDetectionHttpRequest {
    /// The model_id of the LLM to be invoked.
    pub model_id: String,

    /// The prompt to be sent to the LLM.
    pub prompt: String,

    /// The map of detectors to be used, along with their respective parameters, e.g. thresholds.
    pub detectors: HashMap<String, DetectorParams>,

    /// Parameters to be sent to the LLM
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
}

impl GenerationWithDetectionHttpRequest {
    /// Upfront validation of user request
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Validate required parameters
        if self.model_id.is_empty() {
            return Err(ValidationError::Required("model_id".into()));
        }
        if self.prompt.is_empty() {
            return Err(ValidationError::Required("prompt".into()));
        }
        if self.detectors.is_empty() {
            return Err(ValidationError::Required("detectors".into()));
        }

        // Validate detector params
        validate_detector_params(&self.detectors)?;

        Ok(())
    }
}

/// The response format of the /api/v2/text/generation-detection endpoint
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationWithDetectionResult {
    /// Text generated by the LLM
    pub generated_text: String,

    /// Detection results
    pub detections: Vec<DetectionResult>,

    /// Input length
    pub input_token_count: u32,
}

/// Detection format received from detectors
/// This struct does NOT apply to classification endpoints:
/// /api/v1/task/classification-with-text-generation
/// /api/v1/task/server-streaming-classification-with-text-generation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectionResult {
    // The type of detection
    pub detection_type: String,

    // The detection class
    pub detection: String,

    // The confidence level in the detection class
    pub score: f64,

    // Optional evidence block
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<EvidenceObj>>,
}

/// The request format expected in the /api/v2/text/context endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextDocsHttpRequest {
    /// The map of detectors to be used, along with their respective parameters, e.g. thresholds.
    pub detectors: HashMap<String, DetectorParams>,

    /// Content to be sent to detector
    pub content: String,

    /// Content to be sent to detector
    pub context_type: ContextType,

    /// Content to be sent to detector
    pub context: Vec<String>,
}

impl ContextDocsHttpRequest {
    /// Upfront validation of user request
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Validate required parameters
        if self.detectors.is_empty() {
            return Err(ValidationError::Required("detectors".into()));
        }
        if self.content.is_empty() {
            return Err(ValidationError::Required("content".into()));
        }
        if self.context.is_empty() {
            return Err(ValidationError::Required("context".into()));
        }

        // Validate detector params
        validate_detector_params(&self.detectors)?;

        Ok(())
    }
}

/// The response format of the /api/v1/text/task/generation-detection endpoint
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextDocsResult {
    pub detections: Vec<DetectionResult>,
}

/// The request format expected in the /api/v2/text/detect/generated endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatDetectionHttpRequest {
    /// The map of detectors to be used, along with their respective parameters, e.g. thresholds.
    pub detectors: HashMap<String, DetectorParams>,

    // The list of messages to run detections on.
    pub messages: Vec<clients::openai::Message>,
}

impl ChatDetectionHttpRequest {
    /// Upfront validation of user request
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Validate required parameters
        if self.detectors.is_empty() {
            return Err(ValidationError::Required("detectors".into()));
        }
        if self.messages.is_empty() {
            return Err(ValidationError::Required("messages".into()));
        }

        Ok(())
    }

    /// Validates for the "/api/v1/text/chat" endpoint.
    pub fn validate_for_text(&self) -> Result<(), ValidationError> {
        self.validate()?;
        self.validate_messages()?;
        validate_detector_params(&self.detectors)?;

        Ok(())
    }

    /// Validates if message contents are either a string or a content type of type "text"
    fn validate_messages(&self) -> Result<(), ValidationError> {
        for message in &self.messages {
            match &message.content {
                Some(content) => self.validate_content_type(content)?,
                None => {
                    return Err(ValidationError::Invalid(
                        "Message content cannot be empty".into(),
                    ))
                }
            }
        }
        Ok(())
    }

    /// Validates if content type array contains only text messages
    fn validate_content_type(&self, content: &Content) -> Result<(), ValidationError> {
        match content {
            Content::Array(content) => {
                for content_part in content {
                    if content_part.r#type != "text" {
                        return Err(ValidationError::Invalid(
                            "Only content of type text is allowed".into(),
                        ));
                    }
                }
                Ok(())
            }
            Content::String(_) => Ok(()), // if message.content is a string, it is a valid message
        }
    }
}

/// The response format of the /api/v2/text/detection/chat endpoint
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatDetectionResult {
    /// Detection results
    pub detections: Vec<DetectionResult>,
}

/// The request format expected in the /api/v2/text/detect/generated endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectionOnGeneratedHttpRequest {
    /// The prompt to be sent to the LLM.
    pub prompt: String,

    /// The text generated by the LLM.
    pub generated_text: String,

    /// The map of detectors to be used, along with their respective parameters, e.g. thresholds.
    pub detectors: HashMap<String, DetectorParams>,
}

impl DetectionOnGeneratedHttpRequest {
    /// Upfront validation of user request
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Validate required parameters
        if self.prompt.is_empty() {
            return Err(ValidationError::Required("prompt".into()));
        }
        if self.generated_text.is_empty() {
            return Err(ValidationError::Required("generated_text".into()));
        }
        if self.detectors.is_empty() {
            return Err(ValidationError::Required("detectors".into()));
        }

        // Validate detector params
        validate_detector_params(&self.detectors)?;

        Ok(())
    }
}

/// The response format of the /api/v2/text/detection/generated endpoint
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectionOnGenerationResult {
    /// Detection results
    pub detections: Vec<DetectionResult>,
}

/// Validates detector params.
fn validate_detector_params(
    models: &HashMap<String, DetectorParams>,
) -> Result<(), ValidationError> {
    for (model_id, detector_params) in models {
        // Validate threshold is a number, if specified
        if let Some(threshold) = detector_params.get("threshold") {
            if !threshold.is_number() {
                return Err(ValidationError::Invalid(format!(
                    "`threshold` parameter specified for model `{model_id}` must be a number"
                )));
            }
        }
    }
    Ok(())
}

/// Individual evidence object for detection response
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    // Name for the evidence
    pub name: String,
    // Optional, value for the evidence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    // Optional, computed score for the value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

/// High level evidence object for detection response
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceObj {
    // Name for the evidence
    pub name: String,
    // Optional, value for the evidence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    // Optional, omputed score for the value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    // Optional, additional evidence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<Evidence>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate() {
        // Expected OK case
        let request = GuardrailsHttpRequest {
            model_id: "model".to_string(),
            inputs: "The cow jumped over the moon!".to_string(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    masks: Some(vec![(5, 8)]),
                    models: HashMap::new(),
                }),
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::new(),
                }),
            }),
            text_gen_parameters: None,
        };
        assert!(request.validate().is_ok());

        // Masks end same as inputs length - OK
        let request = GuardrailsHttpRequest {
            model_id: "model".to_string(),
            inputs: "The cow jumped over the moon!".to_string(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    masks: Some(vec![(15, 29)]),
                    models: HashMap::new(),
                }),
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::new(),
                }),
            }),
            text_gen_parameters: None,
        };
        assert!(request.validate().is_ok());

        // No model ID
        let request = GuardrailsHttpRequest {
            model_id: "".to_string(),
            inputs: "short".to_string(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    masks: Some(vec![]),
                    models: HashMap::new(),
                }),
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::new(),
                }),
            }),
            text_gen_parameters: None,
        };
        let result = request.validate();
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("`model_id` is required"));

        // No inputs
        let request = GuardrailsHttpRequest {
            model_id: "model".to_string(),
            inputs: "".to_string(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    masks: None,
                    models: HashMap::new(),
                }),
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::new(),
                }),
            }),
            text_gen_parameters: None,
        };
        let result = request.validate();
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("`inputs` is required"));

        // Mask span beyond inputs
        let request = GuardrailsHttpRequest {
            model_id: "model".to_string(),
            inputs: "short".to_string(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    masks: Some(vec![(0, 12)]),
                    models: HashMap::new(),
                }),
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::new(),
                }),
            }),
            text_gen_parameters: None,
        };
        let result = request.validate();
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("invalid masks"));

        // Mask span end less than span start
        let request = GuardrailsHttpRequest {
            model_id: "model".to_string(),
            inputs: "This is ignored anyway!".to_string(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    masks: Some(vec![(12, 8)]),
                    models: HashMap::new(),
                }),
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::new(),
                }),
            }),
            text_gen_parameters: None,
        };
        let result = request.validate();
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("invalid masks"));

        // Validate detector params

        // Valid detector params, threshold is number -- OK
        let mut valid_detector_params = DetectorParams::new();
        valid_detector_params.insert("threshold".into(), 0.2.into());
        let request = GuardrailsHttpRequest {
            model_id: "model".to_string(),
            inputs: "hello".to_string(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    masks: None,
                    models: HashMap::from_iter([("detector1".into(), valid_detector_params)]),
                }),
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::new(),
                }),
            }),
            text_gen_parameters: None,
        };
        assert!(request.validate().is_ok());

        // Invalid detector params, threshold is string -- ERR
        let mut invalid_detector_params = DetectorParams::new();
        invalid_detector_params.insert("threshold".into(), "0.2".into());
        let request = GuardrailsHttpRequest {
            model_id: "model".to_string(),
            inputs: "hello".to_string(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    masks: None,
                    models: HashMap::from_iter([("detector1".into(), invalid_detector_params)]),
                }),
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::new(),
                }),
            }),
            text_gen_parameters: None,
        };
        assert!(request
            .validate()
            .is_err_and(|e| e.to_string().contains("must be a number")));
    }

    #[test]
    fn test_detector_params() -> Result<(), serde_json::Error> {
        let value_json = r#"
        {
            "threshold": 0.2
        }"#;
        let value: DetectorParams = serde_json::from_str(value_json)?;
        assert_eq!(value.threshold(), Some(0.2));
        let value = DetectorParams::new();
        assert_eq!(value.threshold(), None);
        Ok(())
    }
}
