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
use std::error::Error as _;

use axum::{
    Json,
    extract::rejection::JsonRejection,
    response::{IntoResponse, Response},
};
use http::StatusCode;

use crate::{models::ValidationError, orchestrator};

/// High-level errors to return to clients.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    ServiceUnavailable(String),
    #[error("unexpected error occurred while processing request")]
    Unexpected,
    #[error(transparent)]
    JsonExtractorRejection(#[from] JsonRejection),
    #[error("{0}")]
    JsonError(String),
    #[error("unsupported content type: {0}")]
    UnsupportedContentType(String),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

impl From<orchestrator::Error> for Error {
    fn from(value: orchestrator::Error) -> Self {
        use orchestrator::Error::*;
        match value {
            DetectorNotFound(_) | ChunkerNotFound(_) => Self::NotFound(value.to_string()),
            DetectorRequestFailed { ref error, .. }
            | ChunkerRequestFailed { ref error, .. }
            | GenerateRequestFailed { ref error, .. }
            | ChatCompletionRequestFailed { ref error, .. }
            | TokenizeRequestFailed { ref error, .. } => match error.status_code() {
                StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                    Self::Validation(value.to_string())
                }
                StatusCode::NOT_FOUND => Self::NotFound(value.to_string()),
                StatusCode::SERVICE_UNAVAILABLE => Self::ServiceUnavailable(value.to_string()),
                _ => Self::Unexpected,
            },
            JsonError(message) => Self::JsonError(message),
            Validation(message) => Self::Validation(message),
            _ => Self::Unexpected,
        }
    }
}

impl Error {
    pub fn to_json(self) -> serde_json::Value {
        use Error::*;
        let (code, message) = match self {
            Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ServiceUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            UnsupportedContentType(_) => (StatusCode::UNSUPPORTED_MEDIA_TYPE, self.to_string()),
            Unexpected => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            JsonExtractorRejection(json_rejection) => match json_rejection {
                JsonRejection::JsonDataError(e) => {
                    // Get lower-level serde error message
                    let message = e.source().map(|e| e.to_string()).unwrap_or_default();
                    (e.status(), message)
                }
                _ => (json_rejection.status(), json_rejection.body_text()),
            },
            JsonError(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            IoError(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        };
        serde_json::json!({
            "code": code.as_u16(),
            "details": message,
        })
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        use Error::*;
        let (code, message) = match self {
            Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ServiceUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            UnsupportedContentType(_) => (StatusCode::UNSUPPORTED_MEDIA_TYPE, self.to_string()),
            Unexpected => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            JsonExtractorRejection(json_rejection) => match json_rejection {
                JsonRejection::JsonDataError(e) => {
                    // Get lower-level serde error message
                    let message = e.source().map(|e| e.to_string()).unwrap_or_default();
                    (e.status(), message)
                }
                _ => (json_rejection.status(), json_rejection.body_text()),
            },
            JsonError(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            IoError(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        };
        let error = serde_json::json!({
            "code": code.as_u16(),
            "details": message,
        });
        (code, Json(error)).into_response()
    }
}

impl From<ValidationError> for Error {
    fn from(value: ValidationError) -> Self {
        Self::Validation(value.to_string())
    }
}
