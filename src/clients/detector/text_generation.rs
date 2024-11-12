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
use serde::Serialize;
use tracing::{debug, info, instrument};

use super::{DetectorClient, DetectorClientExt, DetectorError, DEFAULT_PORT};
use crate::{
    clients::{create_http_client, http::HttpClientExt, Client, Error, HttpClient},
    config::ServiceConfig,
    health::HealthCheckResult,
    models::{DetectionResult, DetectorParams},
};

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TextGenerationDetectorClient {
    client: HttpClient,
    health_client: Option<HttpClient>,
}

#[cfg_attr(test, faux::methods)]
impl TextGenerationDetectorClient {
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

    fn client(&self) -> &HttpClient {
        &self.client
    }

    #[instrument(skip_all, fields(model_id))]
    pub async fn text_generation(
        &self,
        model_id: &str,
        request: GenerationDetectionRequest,
        headers: HeaderMap,
    ) -> Result<Vec<DetectionResult>, Error> {
        let url = self
            .client
            .base_url()
            .join("/api/v1/text/generation")
            .unwrap();
        info!(?url, "sending generation detector request");
        let response = self
            .post_with_model_id(model_id, url, headers, request)
            .await?;
        if response.status() == StatusCode::OK {
            let response = response.json().await?;
            debug!(?response, "text generation detector response");
            Ok(response)
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
impl Client for TextGenerationDetectorClient {
    fn name(&self) -> &str {
        "text_context_doc_detector"
    }

    async fn health(&self) -> HealthCheckResult {
        if let Some(health_client) = &self.health_client {
            health_client.health().await
        } else {
            self.client.health().await
        }
    }
}

impl DetectorClient for TextGenerationDetectorClient {}

impl HttpClientExt for TextGenerationDetectorClient {
    fn inner(&self) -> &HttpClient {
        self.client()
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
