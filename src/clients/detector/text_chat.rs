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
    clients::{create_http_client, openai::Message, Client, Error, HttpClient},
    config::ServiceConfig,
    health::HealthCheckResult,
    models::{DetectionResult, DetectorParams},
};

const CHAT_DETECTOR_ENDPOINT: &str = "/api/v1/text/chat";

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TextChatDetectorClient {
    client: HttpClient,
    health_client: Option<HttpClient>,
}

#[cfg_attr(test, faux::methods)]
impl TextChatDetectorClient {
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

    pub async fn text_chat(
        &self,
        model_id: &str,
        request: ChatDetectionRequest,
        headers: HeaderMap,
    ) -> Result<Vec<DetectionResult>, Error> {
        let url = self.client.base_url().join(CHAT_DETECTOR_ENDPOINT).unwrap();
        let request = self
            .client
            .post(url)
            .headers(headers)
            .header(DETECTOR_ID_HEADER_NAME, model_id)
            .json(&request);

        tracing::debug!("Request being sent to chat detector: {:?}", request);
        let response = request.send().await?;
        tracing::debug!("Response received from chat detector: {:?}", response);

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
impl Client for TextChatDetectorClient {
    fn name(&self) -> &str {
        "text_chat_detector"
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
/// /api/v1/text/chat endpoint.
// #[cfg_attr(test, derive(PartialEq))]
#[derive(Debug, Serialize)]
pub struct ChatDetectionRequest {
    /// Chat messages to run detection on
    pub messages: Vec<Message>,

    pub detector_params: DetectorParams,
}

impl ChatDetectionRequest {
    pub fn new(messages: Vec<Message>, detector_params: DetectorParams) -> Self {
        Self {
            messages,
            detector_params,
        }
    }
}
