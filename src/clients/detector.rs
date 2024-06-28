use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{create_http_clients, Error, HttpClient};
use crate::config::ServiceConfig;

const DETECTOR_ID_HEADER_NAME: &str = "detector-id";

// For some reason the order matters here. #[cfg_attr(test, derive(Default), faux::create)] doesn't work. (rustc --explain E0560)
#[cfg_attr(test, faux::create, derive(Default))]
#[derive(Clone)]
pub struct DetectorClient {
    clients: HashMap<String, HttpClient>,
}

#[cfg_attr(test, faux::methods)]
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

    pub async fn text_contents(
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

/// Request for text content analysis
/// Results of this request will contain analysis / detection of each of the provided documents
/// in the order they are present in the `contents` object.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(PartialEq))]
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
#[cfg_attr(test, derive(PartialEq))]
pub enum EvidenceType {
    Url,
    Title,
}

/// Source of the evidence e.g. url
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Evidence {
    /// Evidence source
    pub source: String,
}

/// Evidence in response
#[cfg_attr(test, derive(PartialEq))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceObj {
    /// Type field signifying the type of evidence provided
    #[serde(rename = "type")]
    pub r#type: EvidenceType,
    /// Evidence currently only containing source
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
}

/// Response of text content analysis endpoint
#[cfg_attr(test, derive(PartialEq))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentAnalysisResponse {
    /// Start index of detection
    pub start: usize,
    /// End index of detection
    pub end: usize,
    /// Text corresponding to detection
    pub text: String,
    /// Relevant detection class
    pub detection: String,
    /// Detection type or aggregate detection label
    pub detection_type: String,
    /// Score of detection
    pub score: f64,
    /// Optional, any applicable evidences for detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidences: Option<Vec<EvidenceObj>>,
}

impl From<ContentAnalysisResponse> for crate::models::TokenClassificationResult {
    fn from(value: ContentAnalysisResponse) -> Self {
        Self {
            start: value.start as u32,
            end: value.end as u32,
            word: value.text,
            entity: value.detection,
            entity_group: value.detection_type,
            score: value.score,
            token_count: None,
        }
    }
}
