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
    #[error("chat generation request failed for `{id}`: {error}")]
    ChatGenerateRequestFailed { id: String, error: clients::Error },
    #[error("tokenize request failed for `{id}`: {error}")]
    TokenizeRequestFailed { id: String, error: clients::Error },
    #[error("validation error: {0}")]
    Validation(String),
    #[error("{0}")]
    Other(String),
    #[error("cancelled")]
    Cancelled,
    #[error("json deserialization error: {0}")]
    JsonError(String),
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

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::JsonError(value.to_string())
    }
}
