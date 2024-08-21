use crate::{clients::ClientKind, orchestrator};
use axum::http::StatusCode;

/// Errors from external sources that are received and handled by clients
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExternalError {
    #[error("{}", .message)]
    Grpc { code: StatusCode, message: String },
    #[error("{}", .message)]
    Http { code: StatusCode, message: String },
}

impl ExternalError {
    /// Returns status code.
    pub fn status_code(&self) -> StatusCode {
        match self {
            ExternalError::Grpc { code, .. } => *code,
            ExternalError::Http { code, .. } => *code,
        }
    }

    pub fn into_client_error(self, client_id: String) -> Error {
        match self {
            ExternalError::Grpc { code, message } => Error::ClientRequestFailed {
                id: client_id,
                kind: ClientKind::Generation,
                error: ExternalError::Grpc { code, message },
            },
            ExternalError::Http { code, message } => Error::ClientRequestFailed {
                id: client_id,
                kind: ClientKind::Generation,
                error: ExternalError::Http { code, message },
            },
        }
    }
}

impl From<reqwest::Error> for ExternalError {
    fn from(value: reqwest::Error) -> Self {
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

impl From<tonic::Status> for ExternalError {
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

/// Errors created and propagated by the client.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("can not perform {task}, generation client for model with id `{id}` not found running for this service")]
    GenerationModelNotFound { id: String, task: String },
    #[error("can not perform {task}, no generation clients configured for this service")]
    GenerationNotConfigured { task: String },
    #[error("can not perform {task}, detection client with id `{id}` not found running for this service")]
    DetectorNotFound { id: String, task: String },
    #[error("chunker with id `{id}` not found")]
    ChunkerNotFound { id: String },
    #[error("Request to {kind} client with id `{id}` failed: {error}")]
    ClientRequestFailed {
        id: String,
        kind: ClientKind,
        error: ExternalError,
    },
}

impl From<Error> for orchestrator::Error {
    fn from(value: Error) -> Self {
        match value {
            Error::GenerationModelNotFound { .. }
            | Error::GenerationNotConfigured { .. }
            | Error::DetectorNotFound { .. } => {
                orchestrator::Error::ClientRequestCreationFailed { error: value }
            }
            Error::ChunkerNotFound { .. } => {
                orchestrator::Error::UnexpectedClientError { error: value }
            }
            Error::ClientRequestFailed { .. } => {
                orchestrator::Error::ClientRequestFailed { error: value }
            }
        }
    }
}

impl Error {
    /// Default behaviour helper function. If a request could be sent to the client, a tonic::Status
    /// will be returned. Use this function to turn the status into a ClientRequestFailed error.
    pub fn from_status(status: tonic::Status, client_id: String) -> Self {
        ExternalError::from(status).into_client_error(client_id)
    }
}
