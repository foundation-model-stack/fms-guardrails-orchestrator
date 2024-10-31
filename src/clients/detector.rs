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
use hyper::StatusCode;
use reqwest::Response;
use serde::{Deserialize, Serialize};
use url::Url;

pub mod text_contents;
pub use text_contents::*;
pub mod text_chat;
pub use text_chat::*;
pub mod text_context_doc;
pub use text_context_doc::*;
pub mod text_generation;
pub use text_generation::*;

use super::{Error, HttpClient};
use crate::tracing_utils::{trace_context_from_http_response, with_traceparent_header};

const DEFAULT_PORT: u16 = 8080;
const DETECTOR_ID_HEADER_NAME: &str = "detector-id";

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

/// Make a POST request for an HTTP detector client and return the response.
/// Also injects the `traceparent` header from the current span and traces the response.
pub async fn post_with_headers<T: Debug + Serialize>(
    client: HttpClient,
    url: Url,
    request: T,
    headers: HeaderMap,
    model_id: &str,
) -> Result<Response, Error> {
    let mut headers = with_traceparent_header(headers);
    headers.insert(DETECTOR_ID_HEADER_NAME, model_id.parse().unwrap());
    let response = client
        .post(url)
        .headers(headers)
        .json(&request)
        .send()
        .await?;
    trace_context_from_http_response(&response);
    Ok(response)
}
