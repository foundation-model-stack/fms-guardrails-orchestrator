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
use std::collections::HashMap;

use http::HeaderMap;
use opentelemetry::trace::TraceId;

use crate::{
    models::{DetectorParams, TextContentDetectionHttpRequest, TextContentDetectionResult},
    orchestrator::{Error, Orchestrator},
};

use super::Handle;

impl Handle<TextContentDetectionTask> for Orchestrator {
    type Response = TextContentDetectionResult;

    async fn handle(&self, _task: TextContentDetectionTask) -> Result<Self::Response, Error> {
        todo!()
    }
}

#[derive(Debug)]
pub struct TextContentDetectionTask {
    /// Unique identifier of request trace
    pub trace_id: TraceId,
    /// Content to run detection on
    pub content: String,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    // Headermap
    pub headers: HeaderMap,
}

impl TextContentDetectionTask {
    pub fn new(
        trace_id: TraceId,
        request: TextContentDetectionHttpRequest,
        headers: HeaderMap,
    ) -> Self {
        Self {
            trace_id,
            content: request.content,
            detectors: request.detectors,
            headers,
        }
    }
}
