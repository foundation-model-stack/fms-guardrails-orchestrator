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
    clients::openai,
    models::{ChatDetectionHttpRequest, ChatDetectionResult, DetectorParams},
    orchestrator::{Error, Orchestrator},
};

use super::Handle;

impl Handle<ChatDetectionTask> for Orchestrator {
    type Response = ChatDetectionResult;

    async fn handle(&self, _task: ChatDetectionTask) -> Result<Self::Response, Error> {
        todo!()
    }
}

#[derive(Debug)]
pub struct ChatDetectionTask {
    /// Request unique identifier
    pub trace_id: TraceId,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    // Messages to run detection on
    pub messages: Vec<openai::Message>,
    // Headermap
    pub headers: HeaderMap,
}

impl ChatDetectionTask {
    pub fn new(trace_id: TraceId, request: ChatDetectionHttpRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
            detectors: request.detectors,
            messages: request.messages,
            headers,
        }
    }
}
