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
use base64::{Engine as _, engine::general_purpose};
use http::header::CONTENT_TYPE;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use url::Url;
use uuid::Uuid;

use super::{
    Error,
    http::{JSON_CONTENT_TYPE, RequestBody, ResponseBody},
};
use crate::{
    clients::{
        Client, HttpClient, create_http_client,
        openai::{Message, Tool},
    },
    config::{RouterConfig, ServiceConfig},
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

const ROUTER_HANDLE_ENDPOINT: &str = "/ml/v1-private/router/handle";

#[derive(Clone)]
pub struct DetectorClient {
    client: HttpClient,
    health_client: Option<HttpClient>,
    router_client: Option<HttpClient>,
    router_config: Option<RouterConfig>,
    detector_id: String,
    model_id: Option<String>,
}

impl DetectorClient {
    pub async fn new(
        detector_id: String,
        config: &ServiceConfig,
        health_config: Option<&ServiceConfig>,
        router_config: Option<RouterConfig>,
        model_id: Option<String>,
    ) -> Result<Self, Error> {
        let client = create_http_client(DEFAULT_PORT, config).await?;
        let health_client = if let Some(health_config) = health_config {
            Some(create_http_client(DEFAULT_PORT, health_config).await?)
        } else {
            None
        };
        let router_client = if let Some(ref rc) = router_config {
            if rc.enabled {
                let router_service = ServiceConfig::new(rc.hostname.clone(), rc.port);
                Some(create_http_client(rc.port, &router_service).await?)
            } else {
                None
            }
        } else {
            None
        };
        Ok(Self {
            client,
            health_client,
            router_client,
            router_config,
            detector_id,
            model_id,
        })
    }

    fn build_router_headers(
        &self,
        model_name: &str,
        headers: &mut HeaderMap,
    ) -> Result<(String, &HttpClient), Error> {
        let router = self.router_config.as_ref().ok_or_else(|| Error::Http {
            code: StatusCode::INTERNAL_SERVER_ERROR,
            message: "router config must be present when router is enabled".to_string(),
        })?;

        headers.insert(
            "x-model-name",
            model_name.parse().map_err(|e| Error::Http {
                code: StatusCode::BAD_REQUEST,
                message: format!("invalid model name for x-model-name header: {e}"),
            })?,
        );

        headers.insert(
            "x-reply-type",
            router.reply_type.parse().map_err(|e| Error::Http {
                code: StatusCode::BAD_REQUEST,
                message: format!("failed to set x-reply-type header: {e}"),
            })?,
        );

        headers.insert(
            "x-sla-seconds",
            router
                .sla_seconds
                .to_string()
                .parse()
                .map_err(|e| Error::Http {
                    code: StatusCode::BAD_REQUEST,
                    message: format!("failed to set x-sla-seconds header: {e}"),
                })?,
        );

        let transaction_id = if let Some(v) = headers.get("x-global-transaction-id") {
            v.to_str().unwrap_or("").to_string()
        } else {
            let tid = Uuid::new_v4().to_string();
            headers.insert(
                "x-global-transaction-id",
                tid.parse().map_err(|e| Error::Http {
                    code: StatusCode::BAD_REQUEST,
                    message: format!("failed to set x-global-transaction-id header: {e}"),
                })?,
            );
            tid
        };

        headers.insert(
            "x-method",
            "http_pass".parse().map_err(|e| Error::Http {
                code: StatusCode::BAD_REQUEST,
                message: format!("failed to set x-method header: {e}"),
            })?,
        );

        let router_client = self.router_client.as_ref().ok_or_else(|| Error::Http {
            code: StatusCode::INTERNAL_SERVER_ERROR,
            message: "router_client must be present when router is enabled".to_string(),
        })?;

        Ok((transaction_id, router_client))
    }

    async fn post<U: ResponseBody>(
        &self,
        model_id: &str,
        url: Url,
        mut headers: HeaderMap,
        request: impl RequestBody,
    ) -> Result<U, Error> {
        // Check if router is enabled
        let use_router = self.router_config.as_ref().is_some_and(|r| r.enabled);

        if use_router {
            self.post_via_router(model_id, url, headers, request).await
        } else {
            // Original direct HTTP path
            headers.append(DETECTOR_ID_HEADER_NAME, model_id.parse().unwrap());
            headers.append(CONTENT_TYPE, JSON_CONTENT_TYPE);
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
    }

    async fn post_via_router<U: ResponseBody>(
        &self,
        model_id: &str,
        url: Url,
        mut headers: HeaderMap,
        request: impl RequestBody,
    ) -> Result<U, Error> {
        // Use model_id from config if present, otherwise use detector_id
        // This allows detectors that are also models (like Granite Guardian) to route
        // to the correct model queue name
        let queue_model_name = self.model_id.as_ref().unwrap_or(&self.detector_id);

        debug!(
            "Routing detector request via router: detector_id={}, queue_model_name={}",
            model_id, queue_model_name
        );

        let (transaction_id, router_client) =
            self.build_router_headers(queue_model_name, &mut headers)?;

        // Serialize the detector request
        let request_json = serde_json::to_value(&request).map_err(|e| Error::Http {
            code: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to serialize detector request: {e}"),
        })?;

        let request_bytes = serde_json::to_vec(&request_json).map_err(|e| Error::Http {
            code: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to serialize detector request to bytes: {e}"),
        })?;

        // Base64-encode the payload bytes (router expects base64, not array of numbers)
        let payload_base64 = general_purpose::STANDARD.encode(&request_bytes);

        // Build HTTP passthrough payload (same structure as openai.rs lines 247-255)
        let http_pass_payload = serde_json::json!({
            "method": "POST",
            "path": url.path(),
            "headers": {
                "Content-Type": "application/json",
                "x-request-id": transaction_id,
                "detector-id": model_id,
            },
            "payload": payload_base64,
        });

        let router_url = router_client.endpoint(ROUTER_HANDLE_ENDPOINT);
        debug!("Posting to router-sender: {}", router_url);

        // Send to router and get response
        let response = router_client
            .post(router_url, headers.clone(), http_pass_payload)
            .await?;

        let status = response.status();
        match status {
            StatusCode::OK => Ok(response.json().await?),
            _ => Err(response
                .json::<DetectorError>()
                .await
                .unwrap_or(DetectorError {
                    code: status.as_u16(),
                    message: "router request failed".into(),
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

    async fn health(&self, headers: HeaderMap) -> HealthCheckResult {
        if let Some(health_client) = &self.health_client {
            health_client.health(headers).await
        } else {
            self.client.health(headers).await
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
