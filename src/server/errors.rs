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
use serde::{Deserialize, Serialize};

use crate::{models::ValidationError, orchestrator};

/// High-level errors to return to clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Error {
    #[serde(with = "http_serde::status_code")]
    pub code: StatusCode,
    pub details: String,
}

impl Error {
    pub fn code(&self) -> &StatusCode {
        &self.code
    }

    pub fn details(&self) -> &str {
        &self.details
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.details())
    }
}

impl std::error::Error for Error {}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let code = *self.code();
        (code, Json(self)).into_response()
    }
}

impl From<JsonRejection> for Error {
    fn from(value: JsonRejection) -> Self {
        use JsonRejection::*;
        let code = value.status();
        let details = match value {
            JsonDataError(error) => {
                // Get lower-level serde error message
                error.source().map(|e| e.to_string()).unwrap_or_default()
            }
            _ => value.body_text(),
        };
        Self { code, details }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self {
            code: StatusCode::INTERNAL_SERVER_ERROR,
            details: format!("io error: {value}"),
        }
    }
}

impl From<ValidationError> for Error {
    fn from(value: ValidationError) -> Self {
        Self {
            code: StatusCode::UNPROCESSABLE_ENTITY,
            details: value.to_string(),
        }
    }
}

impl From<orchestrator::Error> for Error {
    fn from(value: orchestrator::Error) -> Self {
        use orchestrator::Error::*;
        match value {
            DetectorNotFound(_) | ChunkerNotFound(_) => Self {
                code: StatusCode::NOT_FOUND,
                details: value.to_string(),
            },
            DetectorRequestFailed { ref error, .. }
            | ChunkerRequestFailed { ref error, .. }
            | GenerateRequestFailed { ref error, .. }
            | ChatCompletionRequestFailed { ref error, .. }
            | CompletionRequestFailed { ref error, .. }
            | TokenizeRequestFailed { ref error, .. }
            | Client(ref error) => match error.status_code() {
                // return actual error for subset of errors
                StatusCode::BAD_REQUEST
                | StatusCode::UNPROCESSABLE_ENTITY
                | StatusCode::NOT_FOUND
                | StatusCode::SERVICE_UNAVAILABLE => Self {
                    code: error.status_code(),
                    details: value.to_string(),
                },
                // return generic error for other errors
                _ => Self {
                    code: StatusCode::INTERNAL_SERVER_ERROR,
                    details: "unexpected error occurred while processing request".into(),
                },
            },
            JsonError(message) | Validation(message) => Self {
                code: StatusCode::UNPROCESSABLE_ENTITY,
                details: message,
            },
            _ => Self {
                code: StatusCode::INTERNAL_SERVER_ERROR,
                details: "unexpected error occurred while processing request".into(),
            },
        }
    }
}
