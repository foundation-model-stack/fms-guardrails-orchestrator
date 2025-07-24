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

use std::{collections::BTreeMap, fmt::Debug};

use async_trait::async_trait;
use axum::http::HeaderMap;
use http::header::CONTENT_TYPE;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

use super::{
    Error,
    http::{JSON_CONTENT_TYPE, RequestBody, ResponseBody},
};
use crate::{
    clients::{
        Client, HttpClient, create_http_client,
        openai::{Message, Tool},
    },
    config::ServiceConfig,
    health::HealthCheckResult,
    models::{DetectionResult, DetectorParams, EvidenceObj, Metadata},
};

pub const DEFAULT_PORT: u16 = 8080;
pub const MODEL_HEADER_NAME: &str = "x-model-name";
pub const DETECTOR_ID_HEADER_NAME: &str = "detector-id";
pub const CONTENTS_DETECTOR_ENDPOINT: &str = "/api/v1/text/contents";
pub const CHAT_DETECTOR_ENDPOINT: &str = "/api/v1/text/chat";
pub const CONTEXT_DOC_DETECTOR_ENDPOINT: &str = "/api/v1/text/context/doc";
pub const GENERATION_DETECTOR_ENDPOINT: &str = "/api/v1/text/generation";

#[derive(Clone)]
pub struct DetectorClient {
    client: HttpClient,
    health_client: Option<HttpClient>,
}

impl DetectorClient {
    pub async fn new(
        config: &ServiceConfig,
        health_config: Option<&ServiceConfig>,
    ) -> Result<Self, Error> {
        let client = create_http_client(DEFAULT_PORT, config).await?;
        let health_client = if let Some(health_config) = health_config {
            Some(create_http_client(DEFAULT_PORT, health_config).await?)
        } else {
            None
        };
        Ok(Self {
            client,
            health_client,
        })
    }

    async fn post<U: ResponseBody>(
        &self,
        model_id: &str,
        url: Url,
        mut headers: HeaderMap,
        request: impl RequestBody,
    ) -> Result<U, Error> {
        headers.append(DETECTOR_ID_HEADER_NAME, model_id.parse().unwrap());
        headers.append(CONTENT_TYPE, JSON_CONTENT_TYPE);
        // Header used by a router component, if available
        headers.append(MODEL_HEADER_NAME, model_id.parse().unwrap());

        let response = self.client.post(url, headers, request).await?;

        let status = response.status();
        match status {
            StatusCode::OK => Ok(response.json().await?),
            _ => Err(response
                .json::<DetectorError>()
                .await
                .unwrap_or(DetectorError {
                    code: status.as_u16(),
                    message: "".into(),
                })
                .into()),
        }
    }

    pub async fn text_contents(
        &self,
        model_id: &str,
        request: ContentAnalysisRequest,
        headers: HeaderMap,
    ) -> Result<Vec<Vec<ContentAnalysisResponse>>, Error> {
        let url = self.client.endpoint(CONTENTS_DETECTOR_ENDPOINT);
        info!("sending text content detector request to {}", url);
        self.post(model_id, url, headers, request).await
    }

    pub async fn text_chat(
        &self,
        model_id: &str,
        request: ChatDetectionRequest,
        headers: HeaderMap,
    ) -> Result<Vec<DetectionResult>, Error> {
        let url = self.client.endpoint(CHAT_DETECTOR_ENDPOINT);
        info!("sending text chat detector request to {}", url);
        self.post(model_id, url, headers, request).await
    }

    pub async fn text_context_doc(
        &self,
        model_id: &str,
        request: ContextDocsDetectionRequest,
        headers: HeaderMap,
    ) -> Result<Vec<DetectionResult>, Error> {
        let url = self.client.endpoint(CONTEXT_DOC_DETECTOR_ENDPOINT);
        info!("sending text context doc detector request to {}", url);
        self.post(model_id, url, headers, request).await
    }

    pub async fn text_generation(
        &self,
        model_id: &str,
        request: GenerationDetectionRequest,
        headers: HeaderMap,
    ) -> Result<Vec<DetectionResult>, Error> {
        let url = self.client.endpoint(GENERATION_DETECTOR_ENDPOINT);
        info!("sending text generation detector request to {}", url);
        self.post(model_id, url, headers, request).await
    }
}

#[async_trait]
impl Client for DetectorClient {
    fn name(&self) -> &str {
        "detector"
    }

    async fn health(&self) -> HealthCheckResult {
        if let Some(health_client) = &self.health_client {
            health_client.health().await
        } else {
            self.client.health().await
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

/// Request for text content analysis
/// Results of this request will contain analysis / detection of each of the provided documents
/// in the order they are present in the `contents` object.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContentAnalysisRequest {
    /// Field allowing users to provide list of documents for analysis
    pub contents: Vec<String>,
    /// Detector parameters (available parameters depend on the detector)
    pub detector_params: DetectorParams,
}

impl ContentAnalysisRequest {
    pub fn new(contents: Vec<String>, detector_params: DetectorParams) -> ContentAnalysisRequest {
        ContentAnalysisRequest {
            contents,
            detector_params,
        }
    }
}

/// Response of text content analysis endpoint
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
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
    /// Optional, ID of Detector
    pub detector_id: Option<String>,
    /// Score of detection
    pub score: f64,
    /// Optional, any applicable evidence for detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<EvidenceObj>>,
    // Optional metadata block
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
}

impl From<ContentAnalysisResponse> for crate::models::TokenClassificationResult {
    fn from(value: ContentAnalysisResponse) -> Self {
        Self {
            start: value.start as u32,
            end: value.end as u32,
            word: value.text,
            entity: value.detection,
            entity_group: value.detection_type,
            detector_id: value.detector_id,
            score: value.score,
            token_count: None,
        }
    }
}

/// A struct representing a request to a detector compatible with the
/// /api/v1/text/chat endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct ChatDetectionRequest {
    /// Chat messages to run detection on
    pub messages: Vec<Message>,
    /// Optional list of tool definitions
    pub tools: Vec<Tool>,
    /// Detector parameters (available parameters depend on the detector)
    pub detector_params: DetectorParams,
}

impl ChatDetectionRequest {
    pub fn new(messages: Vec<Message>, tools: Vec<Tool>, detector_params: DetectorParams) -> Self {
        Self {
            messages,
            tools,
            detector_params,
        }
    }
}

/// A struct representing a request to a detector compatible with the
/// /api/v1/text/context/doc endpoint.
#[cfg_attr(test, derive(PartialEq))]
#[derive(Debug, Clone, Serialize)]
pub struct ContextDocsDetectionRequest {
    /// Content to run detection on
    pub content: String,
    /// Type of context being sent
    pub context_type: ContextType,
    /// Context to run detection on
    pub context: Vec<String>,
    /// Detector parameters (available parameters depend on the detector)
    pub detector_params: DetectorParams,
}

/// Enum representing the context type of a detection
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ContextType {
    #[serde(rename = "docs")]
    Document,
    #[serde(rename = "url")]
    Url,
}

impl ContextDocsDetectionRequest {
    pub fn new(
        content: String,
        context_type: ContextType,
        context: Vec<String>,
        detector_params: DetectorParams,
    ) -> Self {
        Self {
            content,
            context_type,
            context,
            detector_params,
        }
    }
}

/// A struct representing a request to a detector compatible with the
/// /api/v1/text/generation endpoint.
#[cfg_attr(test, derive(PartialEq))]
#[derive(Debug, Clone, Serialize)]
pub struct GenerationDetectionRequest {
    /// User prompt sent to LLM
    pub prompt: String,
    /// Text generated from an LLM
    pub generated_text: String,
    /// Detector parameters (available parameters depend on the detector)
    pub detector_params: DetectorParams,
}

impl GenerationDetectionRequest {
    pub fn new(prompt: String, generated_text: String, detector_params: DetectorParams) -> Self {
        Self {
            prompt,
            generated_text,
            detector_params,
        }
    }
}
