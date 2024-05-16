#![allow(unused_qualifications)]

use std::collections::HashMap;

use crate::pb;

pub type DetectorParams = HashMap<String, serde_json::Value>;

/// User request to orchestrator
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, garde::Validate)]
pub struct GuardrailsHttpRequest {
    /// Text generation model ID
    #[serde(rename = "model_id")]
    #[garde(length(min = 1))]
    pub model_id: String,

    /// User prompt/input text to a text generation model
    #[serde(rename = "inputs")]
    #[garde(length(min = 1))]
    pub inputs: String,

    /// Configuration of guardrails models for either or both input to a text generation model
    /// (e.g. user prompt) and output of a text generation model
    #[serde(rename = "guardrail_config")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub guardrail_config: Option<GuardrailsConfig>,

    /// Parameters for text generation
    #[serde(rename = "text_gen_parameters")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
}

/// Configuration of guardrails models for either or both input to a text generation model
/// (e.g. user prompt) and output of a text generation model
#[derive(
    Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, garde::Validate,
)]
pub struct GuardrailsConfig {
    /// Configuration for detection on input to a text generation model (e.g. user prompt)
    #[serde(rename = "input")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub input: Option<GuardrailsConfigInput>,

    /// Configuration for detection on output of a text generation model
    #[serde(rename = "output")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub output: Option<GuardrailsConfigOutput>,
}

impl GuardrailsConfig {
    pub fn input_masks(&self) -> Option<&[(usize, usize)]> {
        self.input.as_ref().and_then(|input| input.masks.as_deref())
    }

    pub fn input_detectors(&self) -> Option<&HashMap<String, DetectorParams>> {
        self.input.as_ref().and_then(|input| input.models.as_ref())
    }

    pub fn output_detectors(&self) -> Option<&HashMap<String, DetectorParams>> {
        self.output
            .as_ref()
            .and_then(|output| output.models.as_ref())
    }
}

/// Configuration for detection on input to a text generation model (e.g. user prompt)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, garde::Validate)]
pub struct GuardrailsConfigInput {
    /// Map of model name to model specific parameters
    #[serde(rename = "models")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(required)] // input field must have `models`
    pub models: Option<HashMap<String, DetectorParams>>,
    /// Vector of spans are in the form of (span_start, span_end) corresponding
    /// to spans of input text on which to run input detection
    #[serde(rename = "masks")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)] // TODO: custom
    pub masks: Option<Vec<(usize, usize)>>,
}

/// Configuration for detection on output of a text generation model
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, garde::Validate)]
pub struct GuardrailsConfigOutput {
    /// Map of model name to model specific parameters
    #[serde(rename = "models")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(required)] // output field must have `models`
    pub models: Option<HashMap<String, DetectorParams>>,
}

/// Parameters for text generation, ref. <https://github.com/IBM/text-generation-inference/blob/main/proto/generation.proto>
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, garde::Validate)]
pub struct GuardrailsTextGenerationParameters {
    // Leave most validation of parameters to downstream text generation servers
    /// Maximum number of new tokens to generate
    #[serde(rename = "max_new_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(range(min = 0))]
    pub max_new_tokens: Option<i32>,

    /// Minimum number of new tokens to generate
    #[serde(rename = "min_new_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(range(min = 0))]
    pub min_new_tokens: Option<i32>,

    /// Truncate to this many input tokens for generation
    #[serde(rename = "truncate_input_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(range(min = 0))]
    pub truncate_input_tokens: Option<i32>,

    /// The high level decoding strategy for picking
    /// tokens during text generation
    #[serde(rename = "decoding_method")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub decoding_method: Option<String>,

    /// Number of highest probability vocabulary tokens to keep for top-k-filtering.
    /// Only applies for sampling mode. When decoding_strategy is set to sample,
    /// only the top_k most likely tokens are considered as candidates for the next generated token.
    #[serde(rename = "top_k")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub top_k: Option<i32>,

    /// Similar to top_k except the candidates to generate the next token are the
    /// most likely tokens with probabilities that add up to at least top_p.
    /// Also known as nucleus sampling. A value of 1.0 is equivalent to disabled.
    #[serde(rename = "top_p")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub top_p: Option<f64>,

    /// Local typicality measures how similar the conditional probability of
    /// predicting a target token next is to the expected conditional
    /// probability of predicting a random token next, given the partial text
    /// already generated
    #[serde(rename = "typical_p")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub typical_p: Option<f64>,

    /// A value used to modify the next-token probabilities in sampling mode.
    /// Values less than 1.0 sharpen the probability distribution, resulting in
    /// "less random" output. Values greater than 1.0 flatten the probability distribution,
    /// resulting in "more random" output. A value of 1.0 has no effect.
    #[serde(rename = "temperature")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(range(min = 0.0))]
    pub temperature: Option<f64>,

    /// Represents the penalty for penalizing tokens that have already been generated
    /// or belong to the context. The value 1.0 means that there is no penalty.
    #[serde(rename = "repetition_penalty")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub repetition_penalty: Option<f64>,

    /// Time limit in milliseconds for text generation to complete
    #[serde(rename = "max_time")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub max_time: Option<f64>,

    /// Parameters to exponentially increase the likelihood of the text generation
    /// terminating once a specified number of tokens have been generated.
    #[serde(rename = "exponential_decay_length_penalty")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub exponential_decay_length_penalty: Option<ExponentialDecayLengthPenalty>,

    /// One or more strings which will cause the text generation to stop if/when
    /// they are produced as part of the output.
    #[serde(rename = "stop_sequences")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub stop_sequences: Option<Vec<String>>,

    /// Random seed used for text generation
    #[serde(rename = "seed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub seed: Option<i32>,

    /// Whether or not to include input text
    #[serde(rename = "preserve_input_text")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub preserve_input_text: Option<bool>,

    /// Whether or not to include input text
    #[serde(rename = "input_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub input_tokens: Option<bool>,

    /// Whether or not to include list of individual generated tokens
    #[serde(rename = "generated_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub generated_tokens: Option<bool>,

    /// Whether or not to include logprob for each returned token
    /// Applicable only if generated_tokens == true and/or input_tokens == true
    #[serde(rename = "token_logprobs")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub token_logprobs: Option<bool>,

    /// Whether or not to include rank of each returned token
    /// Applicable only if generated_tokens == true and/or input_tokens == true
    #[serde(rename = "token_ranks")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub token_ranks: Option<bool>,
}

/// Parameters to exponentially increase the likelihood of the text generation
/// terminating once a specified number of tokens have been generated.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, garde::Validate)]
pub struct ExponentialDecayLengthPenalty {
    /// Start the decay after this number of tokens have been generated
    #[serde(rename = "start_index")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(range(min = 0))]
    pub start_index: Option<i32>,

    /// Factor of exponential decay
    #[serde(rename = "decay_factor")]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub decay_factor: Option<f64>,
}

/// Classification result on text produced by a text generation model, containing
/// information from the original text generation output as well as the result of
/// classification on the generated text.
#[derive(Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct ClassifiedGeneratedTextResult {
    /// Generated text
    #[serde(rename = "generated_text")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_text: Option<String>,

    /// Classification results for input to text generation model and/or
    /// output from the text generation model
    #[serde(rename = "token_classification_results")]
    pub token_classification_results: TextGenTokenClassificationResults,

    /// Why text generation stopped
    #[serde(rename = "finish_reason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    /// Length of sequence of generated tokens
    #[serde(rename = "generated_token_count")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_token_count: Option<i32>,

    /// Random seed used for text generation
    #[serde(rename = "seed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i32>,

    /// Length of input
    #[serde(rename = "input_token_count")]
    pub input_token_count: i32,

    /// Vector of warnings on input detection
    #[serde(rename = "warnings")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<InputWarning>>,

    /// Individual generated tokens and associated details, if requested
    #[serde(rename = "tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    /// Input tokens and associated details, if requested
    #[serde(rename = "input_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,
}

/// Streaming classification result on text produced by a text generation model, containing
/// information from the original text generation output as well as the result of
/// classification on the generated text. Also indicates where in stream is processed.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct ClassifiedGeneratedTextStreamResult {
    #[serde(rename = "generated_text")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_text: Option<String>,

    /// Classification results for input to text generation model and/or
    /// output from the text generation model
    #[serde(rename = "token_classification_results")]
    pub token_classification_results: TextGenTokenClassificationResults,

    /// Why text generation stopped
    #[serde(rename = "finish_reason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    /// Length of sequence of generated tokens
    #[serde(rename = "generated_token_count")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_token_count: Option<i32>,

    /// Random seed used for text generation
    #[serde(rename = "seed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i32>,

    /// Length of input
    #[serde(rename = "input_token_count")]
    pub input_token_count: i32,

    /// Vector of warnings on input detection
    #[serde(rename = "warnings")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<InputWarning>>,

    /// Individual generated tokens and associated details, if requested
    #[serde(rename = "tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    /// Input tokens and associated details, if requested
    #[serde(rename = "input_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,

    /// Result index up to which text is processed
    #[serde(rename = "processed_index")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_index: Option<i32>,

    /// Result start index for processed text
    #[serde(rename = "start_index")]
    pub start_index: i32,
}

/// Results of classification on input to a text generation model (e.g. user prompt)
/// or output of a text generation model
#[derive(Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct TextGenTokenClassificationResults {
    /// Classification results on input to a text generation model
    #[serde(rename = "input")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<TokenClassificationResult>>,

    /// Classification results on output from a text generation model
    #[serde(rename = "output")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<TokenClassificationResult>>,
}

/// Single token classification result
/// NOTE: This is meant to align with the HuggingFace token classification task:
/// <https://huggingface.co/docs/transformers/tasks/token_classification#inference>
/// The field `word` does not necessarily correspond to a single "word",
/// and `entity` may not always be applicable beyond "entity" in the NER
/// (named entity recognition) sense
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TokenClassificationResult {
    /// Beginning/start offset of token
    #[serde(rename = "start")]
    pub start: i32,

    /// End offset of token
    #[serde(rename = "end")]
    pub end: i32,

    /// Text referenced by token
    #[serde(rename = "word")]
    pub word: String,

    /// Predicted relevant class name for the token
    #[serde(rename = "entity")]
    pub entity: String,

    /// Aggregate label, if applicable
    #[serde(rename = "entity_group")]
    pub entity_group: String,

    /// Confidence-like score of this classification prediction in [0, 1]
    #[serde(rename = "score")]
    pub score: f64,

    /// Length of tokens in the text
    #[serde(rename = "token_count")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<i32>,
}

/// Enumeration of reasons why text generation stopped
/// Since this enum's variants do not hold data, we can easily define them as `#[repr(C)]`
/// which helps with FFI.
#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "conversion", derive(frunk_enum_derive::LabelledGenericEnum))]
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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct InputWarning {
    /// Warning reason
    #[serde(rename = "id")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<InputWarningReason>,

    /// Warning message
    #[serde(rename = "message")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Enumeration of warning reasons on input detection
/// Since this enum's variants do not hold data, we can easily define them as `#[repr(C)]`
/// which helps with FFI.
#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "conversion", derive(frunk_enum_derive::LabelledGenericEnum))]
pub enum InputWarningReason {
    /// Unsuitable text detected on input
    #[serde(rename = "UNSUITABLE_INPUT")]
    UnsuitableInput,
}

/// Generated token information
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct GeneratedToken {
    /// Token text
    #[serde(rename = "text")]
    pub text: String,

    /// Logprob (log of normalized probability)
    #[serde(rename = "logprob")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprob: Option<f64>,

    /// One-based rank relative to other tokens
    #[serde(rename = "rank")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<i32>,
}

/// Result of a text generation model
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct GeneratedTextResult {
    /// Generated text
    #[serde(rename = "generated_text")]
    pub generated_text: String,

    /// Length of sequence of generated tokens
    #[serde(rename = "generated_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_tokens: Option<i32>,

    /// Why text generation stopped
    #[serde(rename = "finish_reason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    /// Length of input
    #[serde(rename = "input_token_count")]
    pub input_token_count: i32,

    /// Random seed used for text generation
    #[serde(rename = "seed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i32>,

    /// Individual generated tokens and associated details, if requested
    #[serde(rename = "tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    /// Input tokens and associated details, if requested
    #[serde(rename = "input_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,
}

/// Details on the streaming result of a text generation model
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct TokenStreamDetails {
    /// Why text generation stopped
    #[serde(rename = "finish_reason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    /// Length of sequence of generated tokens
    #[serde(rename = "generated_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_tokens: Option<i32>,

    /// Random seed used for text generation
    #[serde(rename = "seed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i32>,

    /// Length of input
    #[serde(rename = "input_token_count")]
    pub input_token_count: i32,
}

/// Streaming result of a text generation model
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct GeneratedTextStreamResult {
    /// Generated text
    #[serde(rename = "generated_text")]
    pub generated_text: String,

    /// Individual generated tokens and associated details, if requested
    #[serde(rename = "tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    /// Details on the streaming result of a text generation model
    #[serde(rename = "details")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<TokenStreamDetails>,

    /// Streaming result of a text generation model
    #[serde(rename = "input_tokens")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,
}

// TODO: The below errors follow FastAPI concepts esp. for loc
// It may be worth revisiting if the orchestrator without FastAPI
// should be using these error types

/// HTTP validation error
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct HttpValidationError {
    #[serde(rename = "detail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ValidationError>>,
}

/// Validation error
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct ValidationError {
    #[serde(rename = "loc")]
    pub loc: Vec<LocationInner>,

    /// Error message
    #[serde(rename = "msg")]
    pub msg: String,

    /// Error type
    #[serde(rename = "type")]
    pub r#type: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct LocationInner {}

impl From<ExponentialDecayLengthPenalty> for pb::fmaas::decoding_parameters::LengthPenalty {
    fn from(value: ExponentialDecayLengthPenalty) -> Self {
        Self {
            start_index: value.start_index.unwrap_or_default() as u32,
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
            top_k: value.top_k.unwrap_or_default() as u32,
            top_p: value.top_p.unwrap_or_default() as f32,
            typical_p: value.typical_p.unwrap_or_default() as f32,
            seed: value.seed.map(|v| v as u64),
        };
        let stopping = pb::fmaas::StoppingCriteria {
            max_new_tokens: value.max_new_tokens.unwrap_or_default() as u32,
            min_new_tokens: value.min_new_tokens.unwrap_or_default() as u32,
            time_limit_millis: value.max_time.unwrap_or_default() as u32,
            stop_sequences: value.stop_sequences.unwrap_or_default(),
            include_stop_sequence: None,
        };
        let response = pb::fmaas::ResponseOptions {
            input_text: false, // missing?
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
        let truncate_input_tokens = value.truncate_input_tokens.unwrap_or_default() as u32;
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
            rank: Some(value.rank as i32),
        }
    }
}

impl From<pb::caikit_data_model::nlp::GeneratedToken> for GeneratedToken {
    fn from(value: pb::caikit_data_model::nlp::GeneratedToken) -> Self {
        Self {
            text: value.text,
            logprob: Some(value.logprob),
            rank: Some(value.rank as i32),
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
