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
use tracing::instrument;

use super::Handle;
use crate::{
    clients::openai::{CompletionsRequest, CompletionsResponse},
    orchestrator::{Error, Orchestrator},
};

pub mod streaming;
pub mod unary;

impl Handle<CompletionsDetectionTask> for Orchestrator {
    type Response = CompletionsResponse;

    #[instrument(
        name = "completions_detection",
        skip_all,
        fields(trace_id = ?task.trace_id, headers = ?task.headers)
    )]
    async fn handle(&self, task: CompletionsDetectionTask) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        match task.request.stream {
            Some(true) => streaming::handle_streaming(ctx, task).await,
            _ => unary::handle_unary(ctx, task).await,
        }
    }
}

#[derive(Debug)]
pub struct CompletionsDetectionTask {
    /// Trace ID
    pub trace_id: TraceId,
    /// Request
    pub request: CompletionsRequest,
    /// Headers
    pub headers: HeaderMap,
}

impl CompletionsDetectionTask {
    pub fn new(trace_id: TraceId, request: CompletionsRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
            request,
            headers,
        }
    }
}
