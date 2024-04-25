use serde::{Serialize, Deserialize};

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorTaskRequestHttpRequest {
    #[serde(rename = "inputs")]
    pub inputs: DetectorTaskRequestInputs,
    #[serde(rename = "model_id")]
    pub model_id: String,
}

impl DetectorTaskRequestHttpRequest {
    pub fn new(inputs: DetectorTaskRequestInputs, model_id: String) -> DetectorTaskRequestHttpRequest {
        DetectorTaskRequestHttpRequest {
            inputs: inputs,
            model_id: model_id,
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorTaskRequestInputs {
    #[serde(rename = "text")]
    pub text: String,
    #[serde(rename = "parameters", skip_serializing_if = "Option::is_none")]
    pub parameters: Option<std::collections::HashMap<String, DetectorInputParametersValue>>,
}

impl DetectorTaskRequestInputs {
    pub fn new(text: String) -> DetectorTaskRequestInputs {
        DetectorTaskRequestInputs {
            text: text,
            parameters: None,
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorInputParametersValue {
}

impl DetectorInputParametersValue {
    pub fn new() -> DetectorInputParametersValue {
        DetectorInputParametersValue {
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorTaskResponse {
    #[serde(rename = "start", skip_serializing_if = "Option::is_none")]
    pub start: Option<i32>,
    #[serde(rename = "end", skip_serializing_if = "Option::is_none")]
    pub end: Option<i32>,
    #[serde(rename = "word", skip_serializing_if = "Option::is_none")]
    pub word: Option<String>,
    #[serde(rename = "entity", skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    #[serde(rename = "entity_group", skip_serializing_if = "Option::is_none")]
    pub entity_group: Option<String>,
    #[serde(rename = "score", skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(rename = "token_count", skip_serializing_if = "Option::is_none")]
    pub token_count: Option<i32>,
    #[serde(rename = "annotations", default, with = "::serde_with::rust::double_option", skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Option<serde_json::Value>>,
}

impl DetectorTaskResponse {
    pub fn new() -> DetectorTaskResponse {
        DetectorTaskResponse {
            start: None,
            end: None,
            word: None,
            entity: None,
            entity_group: None,
            score: None,
            token_count: None,
            annotations: None,
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorTaskResponseList {
    #[serde(rename = "detectors", skip_serializing_if = "Option::is_none")]
    pub detectors: Option<Vec<DetectorTaskResponse>>,
}

impl DetectorTaskResponseList {
    pub fn new() -> DetectorTaskResponseList {
        DetectorTaskResponseList {
            detectors: None,
        }
    }
}

