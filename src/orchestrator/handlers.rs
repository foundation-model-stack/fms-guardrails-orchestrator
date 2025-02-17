pub mod chat_completions_detection;
pub use chat_completions_detection::ChatCompletionsDetectionTask;
pub mod chat_detection;
pub use chat_detection::ChatDetectionTask;
pub mod classification_with_gen;
pub use classification_with_gen::ClassificationWithGenTask;
pub mod context_docs_detection;
pub use context_docs_detection::ContextDocsDetectionTask;
pub mod streaming_classification_with_gen;
pub use streaming_classification_with_gen::StreamingClassificationWithGenTask;
pub mod streaming_content_detection;
pub use streaming_content_detection::StreamingContentDetectionTask;
pub mod text_content_detection;
pub use text_content_detection::TextContentDetectionTask;
pub mod generation_with_detection;
pub use generation_with_detection::GenerationWithDetectionTask;
pub mod detection_on_generation;
pub use detection_on_generation::DetectionOnGenerationTask;

use super::Error;

pub trait Handle<Task> {
    type Response: Send + 'static;

    async fn handle(&self, task: Task) -> Result<Self::Response, Error>;
}

mod prelude {
    pub use std::{collections::HashMap, sync::Arc};

    pub use http::HeaderMap;
    pub use opentelemetry::trace::TraceId;
    pub use tokio::sync::mpsc;
    pub use tokio_stream::wrappers::ReceiverStream;

    pub use super::*;
    pub use crate::{
        models::*,
        orchestrator::{
            common::{self, ext::*},
            types::*,
            Context, Error, Orchestrator,
        },
    };
}
