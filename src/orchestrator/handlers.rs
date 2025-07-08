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
use http::HeaderMap;
use opentelemetry::trace::TraceId;

pub mod classification_with_gen;
pub use classification_with_gen::ClassificationWithGenTask;
pub mod streaming_classification_with_gen;
pub use streaming_classification_with_gen::StreamingClassificationWithGenTask;
pub mod chat_completions_detection;
pub mod completions_detection;
pub mod streaming_content_detection;
pub use streaming_content_detection::StreamingContentDetectionTask;
pub mod generation_with_detection;
pub use generation_with_detection::GenerationWithDetectionTask;
pub mod chat_detection;
pub use chat_detection::ChatDetectionTask;
pub mod context_docs_detection;
pub use context_docs_detection::ContextDocsDetectionTask;
pub mod detection_on_generation;
pub use detection_on_generation::DetectionOnGenerationTask;
pub mod text_content_detection;
pub use text_content_detection::TextContentDetectionTask;

use super::Error;

/// Implements a task handler.
pub trait Handle<Task> {
    type Response: Send + 'static;

    async fn handle(&self, task: Task) -> Result<Self::Response, Error>; // TODO: Task<R>
}

/// A task.
pub struct Task<R> {
    /// Trace ID
    pub trace_id: TraceId,
    /// Headers
    pub headers: HeaderMap,
    /// Request
    pub request: R,
}

impl<R> Task<R> {
    pub fn new(trace_id: TraceId, headers: HeaderMap, request: R) -> Self {
        Self {
            trace_id,
            headers,
            request,
        }
    }

    pub fn into_parts(self) -> (TraceId, HeaderMap, R) {
        (self.trace_id, self.headers, self.request)
    }
}
