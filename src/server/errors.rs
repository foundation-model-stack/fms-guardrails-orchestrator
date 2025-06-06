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
use serde::ser::SerializeMap;

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

impl Error {
    pub fn code(&self) -> StatusCode {
        use Error::*;
        match self {
            Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            NotFound(_) => StatusCode::NOT_FOUND,
            ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            UnsupportedContentType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Unexpected => StatusCode::INTERNAL_SERVER_ERROR,
            JsonExtractorRejection(json_rejection) => json_rejection.status(),
            JsonError(_) => StatusCode::UNPROCESSABLE_ENTITY,
            IoError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn message(&self) -> String {
        use Error::*;
        match self {
            JsonExtractorRejection(json_rejection) => match json_rejection {
                JsonRejection::JsonDataError(e) => {
                    // Get lower-level serde error message
                    e.source().map(|e| e.to_string()).unwrap_or_default()
                }
                _ => json_rejection.body_text(),
            },
            _ => self.to_string(),
        }
    }
}

impl serde::Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let code = self.code();
        let message = self.message();
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("code", &code.as_u16())?;
        map.serialize_entry("details", &message)?;
        map.end()
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let code = self.code();
        (code, Json(self)).into_response()
    }
}

impl From<ValidationError> for Error {
    fn from(value: ValidationError) -> Self {
        Self::Validation(value.to_string())
    }
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
