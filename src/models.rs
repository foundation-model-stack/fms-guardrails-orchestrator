#![allow(unused_qualifications)]

use validator::Validate;


#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, validator::Validate)]
pub struct GuardrailsHttpRequest {
    #[serde(rename = "model_id")]
    pub model_id: String,

    #[serde(rename = "inputs")]
    pub inputs: String,

    #[serde(rename = "guardrail_config")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub guardrail_config: Option<GuardrailsConfig>,

    #[serde(rename = "text_gen_parameters")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,

}

impl GuardrailsHttpRequest {
    #[allow(clippy::new_without_default)]
    pub fn new(model_id: String, inputs: String, ) -> GuardrailsHttpRequest {
        GuardrailsHttpRequest {
            model_id,
            inputs,
            guardrail_config: None,
            text_gen_parameters: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
pub struct GuardrailsConfig {
    #[serde(rename = "input")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub input: Option<GuardrailsConfigInput>,

    #[serde(rename = "output")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub output: Option<GuardrailsConfigOutput>,

}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
pub struct GuardrailsConfigInput {
    #[serde(rename = "models")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub models: Option<std::collections::HashMap<String, std::collections::HashMap<String, String>>>,

    #[serde(rename = "masks")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub masks: Option<(usize, usize)>
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
pub struct GuardrailsConfigOutput {
    #[serde(rename = "models")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub models: Option<std::collections::HashMap<String, std::collections::HashMap<String, String>>>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
pub struct GuardrailsTextGenerationParameters {
    #[serde(rename = "max_new_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub max_new_tokens: Option<i32>,

    #[serde(rename = "min_new_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub min_new_tokens: Option<i32>,

    #[serde(rename = "truncate_input_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub truncate_input_tokens: Option<i32>,

    #[serde(rename = "decoding_method")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub decoding_method: Option<String>,

    #[serde(rename = "top_k")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub top_k: Option<i32>,

    #[serde(rename = "top_p")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub top_p: Option<f64>,

    #[serde(rename = "typical_p")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub typical_p: Option<f64>,

    #[serde(rename = "temperature")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub temperature: Option<f64>,

    #[serde(rename = "repetition_penalty")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub repetition_penalty: Option<f64>,

    #[serde(rename = "max_time")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub max_time: Option<f64>,

    #[serde(rename = "exponential_decay_length_penalty")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub exponential_decay_length_penalty: Option<ExponentialDecayLengthPenalty>,

    #[serde(rename = "stop_sequences")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,

    #[serde(rename = "seed")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub seed: Option<i32>,

    #[serde(rename = "preserve_input_text")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub preserve_input_text: Option<bool>,

    #[serde(rename = "input_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub input_tokens: Option<bool>,

    #[serde(rename = "generated_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub generated_tokens: Option<bool>,

    #[serde(rename = "token_logprobs")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub token_logprobs: Option<bool>,

    #[serde(rename = "token_ranks")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub token_ranks: Option<bool>,

}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
pub struct ExponentialDecayLengthPenalty {
    #[serde(rename = "start_index")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub start_index: Option<i32>,

    #[serde(rename = "decay_factor")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub decay_factor: Option<f64>,

}


#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct ClassifiedGeneratedTextResult {
    #[serde(rename = "generated_text")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub generated_text: Option<String>,

    #[serde(rename = "token_classification_results")]
    pub token_classification_results: TextGenTokenClassificationResults,

    #[serde(rename = "finish_reason")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    #[serde(rename = "generated_token_count")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub generated_token_count: Option<i32>,

    #[serde(rename = "seed")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub seed: Option<i32>,

    #[serde(rename = "input_token_count")]
    pub input_token_count: i32,

    #[serde(rename = "warnings")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub warnings: Option<Vec<InputWarning>>,

    #[serde(rename = "tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    #[serde(rename = "input_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,

}


impl ClassifiedGeneratedTextResult {
    #[allow(clippy::new_without_default)]
    pub fn new(token_classification_results: TextGenTokenClassificationResults, input_token_count: i32, ) -> ClassifiedGeneratedTextResult {
        ClassifiedGeneratedTextResult {
            generated_text: None,
            token_classification_results,
            finish_reason: None,
            generated_token_count: None,
            seed: None,
            input_token_count,
            warnings: None,
            tokens: None,
            input_tokens: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct ClassifiedGeneratedTextStreamResult {
    #[serde(rename = "generated_text")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub generated_text: Option<String>,

    #[serde(rename = "token_classification_results")]
    pub token_classification_results: TextGenTokenClassificationResults,

    #[serde(rename = "finish_reason")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    #[serde(rename = "generated_token_count")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub generated_token_count: Option<i32>,

    #[serde(rename = "seed")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub seed: Option<i32>,

    #[serde(rename = "input_token_count")]
    pub input_token_count: i32,

    #[serde(rename = "warnings")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub warnings: Option<Vec<InputWarning>>,

    #[serde(rename = "tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    #[serde(rename = "input_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,

    #[serde(rename = "processed_index")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub processed_index: Option<i32>,

    #[serde(rename = "start_index")]
    pub start_index: i32,

}

impl ClassifiedGeneratedTextStreamResult {
    #[allow(clippy::new_without_default)]
    pub fn new(token_classification_results: TextGenTokenClassificationResults, input_token_count: i32, start_index: i32, ) -> ClassifiedGeneratedTextStreamResult {
        ClassifiedGeneratedTextStreamResult {
            generated_text: None,
            token_classification_results,
            finish_reason: None,
            generated_token_count: None,
            seed: None,
            input_token_count,
            warnings: None,
            tokens: None,
            input_tokens: None,
            processed_index: None,
            start_index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct TextGenTokenClassificationResults {
    #[serde(rename = "input")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub input: Option<Vec<TokenClassificationResult>>,

    #[serde(rename = "output")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub output: Option<Vec<TokenClassificationResult>>,

}


impl TextGenTokenClassificationResults {
    #[allow(clippy::new_without_default)]
    pub fn new() -> TextGenTokenClassificationResults {
        TextGenTokenClassificationResults {
            input: None,
            output: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
pub struct TokenClassificationResult {
    #[serde(rename = "start")]
    pub start: i32,

    #[serde(rename = "end")]
    pub end: i32,

    #[serde(rename = "word")]
    pub word: String,

    #[serde(rename = "entity")]
    pub entity: String,

    #[serde(rename = "entity_group")]
    pub entity_group: String,

    #[serde(rename = "score")]
    pub score: f64,

    #[serde(rename = "token_count")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub token_count: Option<i32>,

}

/// Enumeration of values.
/// Since this enum's variants do not hold data, we can easily define them as `#[repr(C)]`
/// which helps with FFI.
#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct InputWarning {
    #[serde(rename = "id")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub id: Option<InputWarningReason>,

    #[serde(rename = "message")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub message: Option<String>,

}

/// Enumeration of values.
/// Since this enum's variants do not hold data, we can easily define them as `#[repr(C)]`
/// which helps with FFI.
#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk_enum_derive::LabelledGenericEnum))]
pub enum InputWarningReason {
    #[serde(rename = "UNSUITABLE_INPUT")]
    UnsuitableInput,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct GeneratedToken {
    #[serde(rename = "text")]
    pub text: String,

    #[serde(rename = "logprob")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub logprob: Option<f64>,

    #[serde(rename = "rank")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub rank: Option<i32>,

}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct GeneratedTextResult {
    #[serde(rename = "generated_text")]
    pub generated_text: String,

    #[serde(rename = "generated_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub generated_tokens: Option<i32>,

    #[serde(rename = "finish_reason")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    #[serde(rename = "input_token_count")]
    pub input_token_count: i32,

    #[serde(rename = "seed")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub seed: Option<i32>,

    #[serde(rename = "tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    #[serde(rename = "input_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct TokenStreamDetails {
    #[serde(rename = "finish_reason")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub finish_reason: Option<FinishReason>,

    #[serde(rename = "generated_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub generated_tokens: Option<i32>,

    #[serde(rename = "seed")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub seed: Option<i32>,

    #[serde(rename = "input_token_count")]
    pub input_token_count: i32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct GeneratedTextStreamResult {
    #[serde(rename = "generated_text")]
    pub generated_text: String,

    #[serde(rename = "tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub tokens: Option<Vec<GeneratedToken>>,

    #[serde(rename = "details")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub details: Option<TokenStreamDetails>,

    #[serde(rename = "input_tokens")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub input_tokens: Option<Vec<GeneratedToken>>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct HttpValidationError {
    #[serde(rename = "detail")]
    #[serde(skip_serializing_if="Option::is_none")]
    pub detail: Option<Vec<ValidationError>>,

}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct ValidationError {
    #[serde(rename = "loc")]
    pub loc: Vec<LocationInner>,

    #[serde(rename = "msg")]
    pub msg: String,

    #[serde(rename = "type")]
    pub r#type: String,

}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, validator::Validate)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct LocationInner {
}