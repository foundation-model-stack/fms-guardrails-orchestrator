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
use crate::orchestrator::ClientKind;
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
    #[error("generation request failed for `{id}`: {error}")]
    GenerateRequestFailed { id: String, error: clients::Error },
    #[error("chat generation request failed for `{id}`: {error}")]
    ChatGenerateRequestFailed { id: String, error: clients::Error },
    #[error("tokenize request failed for `{id}`: {error}")]
    TokenizeRequestFailed { id: String, error: clients::Error },
    #[error("{0}")]
    Other(String),
    #[error("cancelled")]
    Cancelled,
}

impl Error {
    /// Helper function that logs an error level trace event if the given client error is internal,
    /// and transforms any incoming client error into an HTTP error ready to be returned to the user.
    pub(crate) fn handle_client_error(
        error: clients::Error,
        client_kind: ClientKind,
        client_id: &str,
    ) -> Self {
        debug!(
            ?error,
            "error received from {:?} client {}", client_kind, client_id
        );
        if matches!(error, clients::Error::Internal { .. }) {
            error!("internal client error: {}", error);
        }
        let id = client_id.to_string();
        let error = error.into_http();
        match client_kind {
            ClientKind::Chunker => Self::ChunkerRequestFailed { id, error },
            ClientKind::Detector => Self::DetectorRequestFailed { id, error },
            ClientKind::Generation => Self::GenerateRequestFailed { id, error },
            ClientKind::ChatGeneration => Self::ChatGenerateRequestFailed { id, error },
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
