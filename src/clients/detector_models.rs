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
    #[serde(rename = "start",)]
    pub start: i32,
    #[serde(rename = "end",)]
    pub end: i32,
    #[serde(rename = "text",)]
    pub text: String,
    #[serde(rename = "detection",)]
    pub detection: String,
    #[serde(rename = "detection_type",)]
    pub detection_type: String,
    #[serde(rename = "score",)]
    pub score: f64,
}

impl DetectorTaskResponse {
    pub fn new(start: i32, end: i32, text: String, detection: String, detection_type: String, score: f64) -> DetectorTaskResponse {
        DetectorTaskResponse {
            start: start,
            end: end,
            text: text,
            detection: detection,
            detection_type: detection_type,
            score: score,
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

