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

    pub async fn analyze_contents(
        &self,
        model_id: &str,
        request: ContentAnalysisRequest,
    ) -> Result<Vec<Vec<ContentAnalysisResponse>>, Error> {
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

/// Results of this request will contain analysis / detection of each of the provided documents in the order they are present in the `contents` object.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentAnalysisRequest {
    /// Field allowing users to provide list of documents for analysis
    pub contents: Vec<String>,
}

impl ContentAnalysisRequest {
    pub fn new(contents: Vec<String>) -> ContentAnalysisRequest {
        ContentAnalysisRequest { contents }
    }
}

/// Evidence type
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceType {
    Url,
    Title,
}

/// Source of the evidence e.g. url
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    /// Evidence source
    pub source: String,
}

/// Evidence in response
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceObj {
    /// Type field signifying the type of evidence provided
    #[serde(rename = "type")]
    pub r#type: EvidenceType,
    /// Evidence currently only containing source
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentAnalysisResponse {
    pub start: usize,
    pub end: usize,
    pub detection: String,
    pub detection_type: String,
    pub score: f64,
    // TODO: confirm plurality
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidences: Option<EvidenceObj>,
}

// impl From<Detection> for crate::models::TokenClassificationResult {
//     fn from(value: ContentAnalysisResponse) -> Self {
//         Self {
//             start: value.start as u32,
//             end: value.end as u32,
//             word: value.text, // where to get this now??
//             entity: value.detection,
//             entity_group: value.detection_type,
//             score: value.score,
//             token_count: None,
//         }
//     }
// }
