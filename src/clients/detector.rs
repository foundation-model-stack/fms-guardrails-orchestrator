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

use std::collections::HashMap;

use hyper::StatusCode;
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
            .await?;
        if response.status() == StatusCode::OK {
            Ok(response.json().await?)
        } else {
            let mut error = response.json::<DetectorError>().await.unwrap();
            error.message = format!("detector `{0}`: {1}", model_id, error.message);
            Err(error.into())
        }
    }
}

/// Request for text content analysis
/// Results of this request will contain analysis / detection of each of the provided documents
/// in the order they are present in the `contents` object.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceType {
    Url,
    Title,
}

/// Source of the evidence e.g. url
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    /// Evidence source
    pub source: String,
}

/// Evidence in response
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EvidenceObj {
    /// Type field signifying the type of evidence provided
    #[serde(rename = "type")]
    pub r#type: EvidenceType,
    /// Evidence currently only containing source
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
}

/// Response of text content analysis endpoint
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct DetectorError {
    pub code: u16,
    pub message: String,
}

impl From<DetectorError> for Error {
    fn from(error: DetectorError) -> Self {
        Error::Http {
            code: StatusCode::from_u16(error.code).unwrap(),
            message: error.message,
        }
    }
}
