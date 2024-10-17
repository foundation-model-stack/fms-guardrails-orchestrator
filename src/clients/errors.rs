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

use hyper::StatusCode;
use tracing::error;

/// Client errors.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("{}", .message)]
    Grpc { code: StatusCode, message: String },
    #[error("{}", .message)]
    Http { code: StatusCode, message: String },
    #[error("model not found: {model_id}")]
    ModelNotFound { model_id: String },
}

impl Error {
    /// Returns status code.
    pub fn status_code(&self) -> StatusCode {
        match self {
            // Return equivalent http status code for grpc status code
            Error::Grpc { code, .. } => *code,
            // Return http status code for error responses
            // and 500 for other errors
            Error::Http { code, .. } => *code,
            // Return 404 for model not found
            Error::ModelNotFound { .. } => StatusCode::NOT_FOUND,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        // Log lower level source of error.
        // Examples:
        // 1. client error (Connect) // Cases like connection error, wrong port etc.
        // 2. client error (SendRequest) // Cases like cert issues
        error!(
            "http request failed. Source: {}",
            value.source().unwrap().to_string()
        );
        // Return http status code for error responses
        // and 500 for other errors
        let code = match value.status() {
            Some(code) => code,
            None => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::Http {
            code,
            message: value.to_string(),
        }
    }
}

impl From<tonic::Status> for Error {
    fn from(value: tonic::Status) -> Self {
        Self::Grpc {
            code: grpc_to_http_code(value.code()),
            message: value.message().to_string(),
        }
    }
}

/// Returns equivalent http status code for grpc status code
pub fn grpc_to_http_code(value: tonic::Code) -> StatusCode {
    use tonic::Code::*;
    match value {
        InvalidArgument => StatusCode::BAD_REQUEST,
        Internal => StatusCode::INTERNAL_SERVER_ERROR,
        NotFound => StatusCode::NOT_FOUND,
        DeadlineExceeded => StatusCode::REQUEST_TIMEOUT,
        Unimplemented => StatusCode::NOT_IMPLEMENTED,
        Unauthenticated => StatusCode::UNAUTHORIZED,
        PermissionDenied => StatusCode::FORBIDDEN,
        Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        Ok => StatusCode::OK,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
