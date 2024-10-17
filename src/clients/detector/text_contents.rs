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

use async_trait::async_trait;
use hyper::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use super::{DetectorError, DEFAULT_PORT, DETECTOR_ID_HEADER_NAME};
use crate::{
    clients::{create_http_client, Client, Error, HttpClient},
    config::ServiceConfig,
    health::HealthCheckResult,
};

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TextContentsDetectorClient {
    client: HttpClient,
    health_client: Option<HttpClient>,
}

#[cfg_attr(test, faux::methods)]
impl TextContentsDetectorClient {
    pub async fn new(config: &ServiceConfig, health_config: Option<&ServiceConfig>) -> Self {
        let client = create_http_client(DEFAULT_PORT, config).await;
        let health_client = if let Some(health_config) = health_config {
            Some(create_http_client(DEFAULT_PORT, health_config).await)
        } else {
            None
        };
        Self {
            client,
            health_client,
        }
    }

    pub async fn text_contents(
        &self,
        model_id: &str,
        request: ContentAnalysisRequest,
        headers: HeaderMap,
    ) -> Result<Vec<Vec<ContentAnalysisResponse>>, Error> {
        let url = self
            .client
            .base_url()
            .join("/api/v1/text/contents")
            .unwrap();
        let response = self
            .client
            .post(url)
            .headers(headers)
            .header(DETECTOR_ID_HEADER_NAME, model_id)
            .json(&request)
            .send()
            .await?;
        if response.status() == StatusCode::OK {
            Ok(response.json().await?)
        } else {
            let code = response.status().as_u16();
            let error = response
                .json::<DetectorError>()
                .await
                .unwrap_or(DetectorError {
                    code,
                    message: "".into(),
                });
            Err(error.into())
        }
    }
}

#[cfg_attr(test, faux::methods)]
#[async_trait]
impl Client for TextContentsDetectorClient {
    fn name(&self) -> &str {
        "text_contents_detector"
    }

    async fn health(&self) -> HealthCheckResult {
        if let Some(health_client) = &self.health_client {
            health_client.health().await
        } else {
            self.client.health().await
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
    /// Optional, any applicable evidence for detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<EvidenceObj>>,
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

/// Evidence
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    /// Evidence name
    pub name: String,
    /// Optional, evidence value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Optional, score for evidence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

/// Evidence in response
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EvidenceObj {
    /// Evidence name
    pub name: String,
    /// Optional, evidence value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Optional, score for evidence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Optional, evidence on evidence value
    // Evidence nesting should likely not go beyond this
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<Evidence>>,
}
