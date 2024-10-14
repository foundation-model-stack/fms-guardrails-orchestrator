use async_trait::async_trait;
use hyper::{HeaderMap, StatusCode};
use serde::Serialize;

use super::{DetectorError, DEFAULT_PORT, DETECTOR_ID_HEADER_NAME};
use crate::{
    clients::{create_http_client, openai::Message, Client, Error, HttpClient},
    config::ServiceConfig,
    health::HealthCheckResult,
    models::DetectionResult,
};

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
        let url = self.client.base_url().join("/api/v1/text/chat").unwrap();
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
}

impl ChatDetectionRequest {
    pub fn new(messages: Vec<Message>) -> Self {
        Self { messages }
    }
}
