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

use super::{DetectorError, DEFAULT_PORT, DETECTOR_ID_HEADER_NAME};
use crate::{
    clients::{create_http_client, Client, Error, HttpClient},
    config::ServiceConfig,
    health::HealthCheckResult,
    models::DetectionResult,
};

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TextGenerationDetectorClient {
    client: HttpClient,
    health_client: Option<HttpClient>,
}

#[cfg_attr(test, faux::methods)]
impl TextGenerationDetectorClient {
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

/// A struct representing a request to a detector compatible with the
/// /api/v1/text/generation endpoint.
#[cfg_attr(test, derive(PartialEq))]
#[derive(Debug, Serialize)]
pub struct GenerationDetectionRequest {
    /// User prompt sent to LLM
    pub prompt: String,

    /// Text generated from an LLM
    pub generated_text: String,
}

impl GenerationDetectionRequest {
    pub fn new(prompt: String, generated_text: String) -> Self {
        Self {
            prompt,
            generated_text,
        }
    }
}
