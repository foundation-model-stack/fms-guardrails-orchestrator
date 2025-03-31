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
    clients::detector::ContextType,
    models::{ContextDocsHttpRequest, ContextDocsResult, DetectorParams},
    orchestrator::{Error, Orchestrator},
};

use super::Handle;

impl Handle<ContextDocsDetectionTask> for Orchestrator {
    type Response = ContextDocsResult;

    async fn handle(&self, _task: ContextDocsDetectionTask) -> Result<Self::Response, Error> {
        todo!()
    }
}

#[derive(Debug)]
pub struct ContextDocsDetectionTask {
    /// Unique identifier of request trace
    pub trace_id: TraceId,
    /// Content to run detection on
    pub content: String,
    /// Context type
    pub context_type: ContextType,
    /// Context
    pub context: Vec<String>,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    // Headermap
    pub headers: HeaderMap,
}

impl ContextDocsDetectionTask {
    pub fn new(trace_id: TraceId, request: ContextDocsHttpRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
            content: request.content,
            context_type: request.context_type,
            context: request.context,
            detectors: request.detectors,
            headers,
        }
    }
}
