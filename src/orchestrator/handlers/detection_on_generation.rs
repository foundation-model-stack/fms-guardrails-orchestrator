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
    models::{DetectionOnGeneratedHttpRequest, DetectionOnGenerationResult, DetectorParams},
    orchestrator::{Error, Orchestrator},
};

use super::Handle;

impl Handle<DetectionOnGenerationTask> for Orchestrator {
    type Response = DetectionOnGenerationResult;

    async fn handle(&self, _task: DetectionOnGenerationTask) -> Result<Self::Response, Error> {
        todo!()
    }
}

#[derive(Debug)]
pub struct DetectionOnGenerationTask {
    /// Unique identifier of request trace
    pub trace_id: TraceId,
    /// User prompt to be sent to the LLM
    pub prompt: String,
    /// Text generated by the LLM
    pub generated_text: String,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    // Headermap
    pub headers: HeaderMap,
}

impl DetectionOnGenerationTask {
    pub fn new(
        trace_id: TraceId,
        request: DetectionOnGeneratedHttpRequest,
        headers: HeaderMap,
    ) -> Self {
        Self {
            trace_id,
            prompt: request.prompt,
            generated_text: request.generated_text,
            detectors: request.detectors,
            headers,
        }
    }
}
