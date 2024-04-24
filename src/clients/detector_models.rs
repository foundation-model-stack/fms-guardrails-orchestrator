

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorTaskRequestHttpRequest {
    #[serde(rename = "inputs", skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Box<models::DetectorTaskRequestInputs>>,
    #[serde(rename = "model_id", skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

impl DetectorTaskRequestHttpRequest {
    pub fn new() -> DetectorTaskRequestHttpRequest {
        DetectorTaskRequestHttpRequest {
            inputs: None,
            model_id: None,
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorTaskRequestInputs {
    #[serde(rename = "text", skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "parameters", skip_serializing_if = "Option::is_none")]
    pub parameters: Option<std::collections::HashMap<String, models::DetectorInputParametersValue>>,
}

impl DetectorTaskRequestInputs {
    pub fn new() -> DetectorTaskRequestInputs {
        DetectorTaskRequestInputs {
            text: None,
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
    pub detectors: Option<Vec<models::DetectorTaskResponse>>,
}

impl DetectorTaskResponseList {
    pub fn new() -> DetectorTaskResponseList {
        DetectorTaskResponseList {
            detectors: None,
        }
    }
}

