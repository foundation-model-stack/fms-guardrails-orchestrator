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
        use tonic::Code::*;
        // Return equivalent http status code for grpc status code
        let code = match value.code() {
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
        };
        Self::Grpc {
            code,
            message: value.message().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientCode {
    Http(StatusCode),
    Grpc(tonic::Code),
}

impl std::fmt::Display for ClientCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientCode::Http(code) => write!(f, "HTTP {}", code),
            ClientCode::Grpc(code) => write!(f, "gRPC {:?} {}", code, code),
        }
    }
}
