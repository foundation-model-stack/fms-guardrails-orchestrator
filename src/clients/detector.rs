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

use std::fmt::Debug;

use axum::http::HeaderMap;
use http::header::CONTENT_TYPE;
use hyper::StatusCode;
use serde::Deserialize;
use tracing::instrument;
use url::Url;

use super::{
    Error,
    http::{HttpClientExt, RequestBody, ResponseBody},
};

pub mod text_contents;
pub use text_contents::*;
pub mod text_chat;
pub use text_chat::*;
pub mod text_context_doc;
pub use text_context_doc::*;
pub mod text_generation;
pub use text_generation::*;

const DEFAULT_PORT: u16 = 8080;
pub const DETECTOR_ID_HEADER_NAME: &str = "detector-id";
const MODEL_HEADER_NAME: &str = "x-model-name";

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

/// This trait should be implemented by all detectors.
/// If the detector has an HTTP client (currently all detector clients are HTTP) this trait will
/// implicitly extend the client with an HTTP detector specific post function.
pub trait DetectorClient {}

/// Provides a helper extension for HTTP detector clients.
pub trait DetectorClientExt: HttpClientExt {
    /// Wraps the post function with extra detector functionality
    /// (detector id header injection & error handling)
    async fn post_to_detector<U: ResponseBody>(
        &self,
        model_id: &str,
        url: Url,
        headers: HeaderMap,
        request: impl RequestBody,
    ) -> Result<U, Error>;

    /// Wraps call to inner HTTP client endpoint function.
    fn endpoint(&self, path: &str) -> Url;
}

impl<C: DetectorClient + HttpClientExt> DetectorClientExt for C {
    #[instrument(skip_all, fields(model_id, url))]
    async fn post_to_detector<U: ResponseBody>(
        &self,
        model_id: &str,
        url: Url,
        headers: HeaderMap,
        request: impl RequestBody,
    ) -> Result<U, Error> {
        let mut headers = headers;
        headers.append(DETECTOR_ID_HEADER_NAME, model_id.parse().unwrap());
        headers.append(CONTENT_TYPE, "application/json".parse().unwrap());
        // Header used by a router component, if available
        headers.append(MODEL_HEADER_NAME, model_id.parse().unwrap());

        let response = self.inner().post(url, headers, request).await?;

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

    fn endpoint(&self, path: &str) -> Url {
        self.inner().endpoint(path)
    }
}
