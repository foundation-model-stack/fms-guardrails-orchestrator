use async_trait::async_trait;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

use super::{DetectorError, DETECTOR_ID_HEADER_NAME};
use crate::{
    clients::{Client, Error, HttpClient},
    health::HealthCheckResult,
    models::{DetectionResult, DetectorParams},
};

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TextContextDocDetectorClient {
    client: HttpClient,
}

#[cfg_attr(test, faux::methods)]
impl TextContextDocDetectorClient {
    pub fn new(client: HttpClient) -> Self {
        Self { client }
    }

    pub async fn text_context_docs(
        &self,
        model_id: &str,
        request: ContextDocsDetectionRequest,
    ) -> Result<Vec<DetectionResult>, Error> {
        let url = self
            .client
            .base_url()
            .join("/api/v1/text/context/doc")
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
impl Client for TextContextDocDetectorClient {
    fn name(&self) -> &str {
        "text_context_doc_detector"
    }

    async fn health(&self) -> HealthCheckResult {
        self.client.health().await
    }
}

/// A struct representing a request to a detector compatible with the
/// /api/v1/text/context/doc endpoint.
#[cfg_attr(test, derive(PartialEq))]
#[derive(Debug, Serialize)]
pub struct ContextDocsDetectionRequest {
    /// Content to run detection on
    pub content: String,

    /// Type of context being sent
    pub context_type: ContextType,

    /// Context to run detection on
    pub context: Vec<String>,

    // Detector Params
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
