use serde::{Serialize, Deserialize};

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorTaskRequestHttpRequest {
    #[serde(rename = "text")]
    pub text: String,
    #[serde(rename = "parameters", skip_serializing_if = "Option::is_none")]
    pub parameters: Option<std::collections::HashMap<String, DetectorInputParametersValue>>,
}

impl DetectorTaskRequestHttpRequest {
    pub fn new(text: String) -> DetectorTaskRequestHttpRequest {
        DetectorTaskRequestHttpRequest {
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
    #[serde(rename = "text", skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "detection", skip_serializing_if = "Option::is_none")]
    pub detection: Option<String>,
    #[serde(rename = "detection_type", skip_serializing_if = "Option::is_none")]
    pub detection_type: Option<String>,
    #[serde(rename = "score", skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

impl DetectorTaskResponse {
    pub fn new() -> DetectorTaskResponse {
        DetectorTaskResponse {
            start: None,
            end: None,
            text: None,
            detection: None,
            detection_type: None,
            score: None,
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorTaskResponseList {
    #[serde(rename = "detections", skip_serializing_if = "Option::is_none")]
    pub detections: Option<Vec<DetectorTaskResponse>>,
}

impl DetectorTaskResponseList {
    pub fn new() -> DetectorTaskResponseList {
        DetectorTaskResponseList {
            detections: None,
        }
    }
}

