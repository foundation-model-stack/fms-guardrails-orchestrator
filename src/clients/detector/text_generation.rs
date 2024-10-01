use async_trait::async_trait;
use hyper::StatusCode;
use serde::Serialize;

use super::{DetectorError, DETECTOR_ID_HEADER_NAME};
use crate::{
    clients::{Client, Error, HttpClient},
    health::HealthCheckResult,
    models::DetectionResult,
};

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TextGenerationDetectorClient {
    client: HttpClient,
}

#[cfg_attr(test, faux::methods)]
impl TextGenerationDetectorClient {
    pub fn new(client: HttpClient) -> Self {
        Self { client }
    }

    pub async fn text_generation(
        &self,
        model_id: &str,
        request: GenerationDetectionRequest,
    ) -> Result<Vec<DetectionResult>, Error> {
        let url = self
            .client
            .base_url()
            .join("/api/v1/text/generation")
            .unwrap();
        let response = self
            .client
            .post(url)
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
        self.client.health().await
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
