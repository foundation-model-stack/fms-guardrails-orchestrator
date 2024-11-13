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

use crate::clients;
use tracing::{debug, error};

/// Orchestrator errors.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Client(#[from] clients::Error),
    #[error("detector `{0}` not found")]
    DetectorNotFound(String),
    #[error("detector request failed for `{id}`: {error}")]
    DetectorRequestFailed { id: String, error: clients::Error },
    #[error("chunker request failed for `{id}`: {error}")]
    ChunkerRequestFailed { id: String, error: clients::Error },
    #[error("generate request failed for `{id}`: {error}")]
    GenerateRequestFailed { id: String, error: clients::Error },
    #[error("tokenize request failed for `{id}`: {error}")]
    TokenizeRequestFailed { id: String, error: clients::Error },
    #[error("{0}")]
    Other(String),
    #[error("cancelled")]
    Cancelled,
}

impl Error {
    /// Helper function that logs an error level trace event if the given client error is internal.
    fn handle_internal_client_error(error: &clients::Error, from_client: String) {
        debug!(?error, "error received from {}", from_client);
        if matches!(error, clients::Error::Internal { .. }) {
            error!("internal client error: {}", error);
        }
    }

    /// Logs error if internal and transforms error into HTTP wrapped in `DetectorRequestFailed`
    pub fn handle_detector_error(id: String, error: clients::Error) -> Self {
        Self::handle_internal_client_error(&error, format!("detector client with id: {}", id));
        Error::DetectorRequestFailed {
            id,
            error: error.into_http(),
        }
    }

    /// Logs error if internal and transforms error into HTTP wrapped in `TokenizeRequestFailed`
    pub fn handle_tokenize_error(id: String, error: clients::Error) -> Self {
        Self::handle_internal_client_error(
            &error,
            format!("tokenization in generation client with id: {}", id),
        );
        Error::TokenizeRequestFailed {
            id,
            error: error.into_http(),
        }
    }

    /// Logs error if internal and transforms error into HTTP wrapped in `GenerateRequestFailed`
    pub fn handle_generate_error(id: String, error: clients::Error) -> Self {
        Self::handle_internal_client_error(&error, format!("generation client with id: {}", id));
        Error::GenerateRequestFailed {
            id,
            error: error.into_http(),
        }
    }

    /// Logs error if internal and transforms error into HTTP wrapped in `ChunkerRequestFailed`
    pub fn handle_chunker_error(id: String, error: clients::Error) -> Self {
        Self::handle_internal_client_error(&error, format!("chunker client with id {}", id));
        Error::ChunkerRequestFailed {
            id,
            error: error.into_http(),
        }
    }
}

impl From<tokio::task::JoinError> for Error {
    fn from(error: tokio::task::JoinError) -> Self {
        if error.is_cancelled() {
            Self::Cancelled
        } else {
            Self::Other(format!("task panicked: {error}"))
        }
    }
}
