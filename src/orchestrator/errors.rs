use crate::clients;

/// Orchestrator errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("detector not found: {detector_id}")]
    DetectorNotFound { detector_id: String },
    #[error("detector request failed for detector_id={detector_id}: {error}")]
    DetectorRequestFailed {
        detector_id: String,
        error: clients::Error,
    },
    #[error("chunker request failed for chunker_id={chunker_id}: {error}")]
    ChunkerRequestFailed {
        chunker_id: String,
        error: clients::Error,
    },
    #[error("generate request failed for model_id={model_id}: {error}")]
    GenerateRequestFailed {
        model_id: String,
        error: clients::Error,
    },
    #[error("tokenize request failed for model_id={model_id}: {error}")]
    TokenizeRequestFailed {
        model_id: String,
        error: clients::Error,
    },
    #[error("task cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
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
