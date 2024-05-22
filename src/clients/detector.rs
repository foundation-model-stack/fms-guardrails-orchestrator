use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{create_http_clients, Error, HttpClient};
use crate::config::ServiceConfig;

const DETECTOR_ID_HEADER_NAME: &str = "detector-id";

#[derive(Clone)]
pub struct DetectorClient {
    clients: HashMap<String, HttpClient>,
}

impl DetectorClient {
    pub async fn new(default_port: u16, config: &[(String, ServiceConfig)]) -> Self {
        let clients: HashMap<String, HttpClient> = create_http_clients(default_port, config).await;
        Self { clients }
    }

    fn client(&self, model_id: &str) -> Result<HttpClient, Error> {
        Ok(self
            .clients
            .get(model_id)
            .ok_or_else(|| Error::ModelNotFound {
                model_id: model_id.to_string(),
            })?
            .clone())
    }

    pub async fn classify(
        &self,
        model_id: &str,
        request: DetectorRequest,
    ) -> Result<DetectorResponse, Error> {
        let client = self.client(model_id)?;
        let url = client.base_url().as_str();
        let response = client
            .post(url)
            .header(DETECTOR_ID_HEADER_NAME, model_id)
            .json(&request)
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorRequest {
    pub text: String,
    pub parameters: Option<HashMap<String, serde_json::Value>>,
}

impl DetectorRequest {
    pub fn new(text: String, parameters: HashMap<String, serde_json::Value>) -> Self {
        let parameters = if parameters.is_empty() {
            None
        } else {
            Some(parameters)
        };
        Self { text, parameters }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub detection: String,
    pub detection_type: String,
    pub score: f64,
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectorResponse {
    pub detections: Vec<Detection>,
}

impl From<Detection> for crate::models::TokenClassificationResult {
    fn from(value: Detection) -> Self {
        Self {
            start: value.start as i32,
            end: value.end as i32,
            word: value.text,
            entity: value.detection,
            entity_group: value.detection_type,
            score: value.score,
            token_count: None,
        }
    }
}
