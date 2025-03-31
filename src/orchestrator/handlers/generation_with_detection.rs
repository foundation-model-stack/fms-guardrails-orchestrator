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
    models::{
        DetectorParams, GenerationWithDetectionHttpRequest, GenerationWithDetectionResult,
        GuardrailsTextGenerationParameters,
    },
    orchestrator::{Error, Orchestrator},
};

use super::Handle;

impl Handle<GenerationWithDetectionTask> for Orchestrator {
    type Response = GenerationWithDetectionResult;

    async fn handle(&self, _task: GenerationWithDetectionTask) -> Result<Self::Response, Error> {
        todo!()
    }
}

#[derive(Debug)]
pub struct GenerationWithDetectionTask {
    /// Unique identifier of request trace
    pub trace_id: TraceId,
    /// Model ID of the LLM
    pub model_id: String,
    /// User prompt to be sent to the LLM
    pub prompt: String,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    /// LLM Parameters
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
    // Headermap
    pub headers: HeaderMap,
}

impl GenerationWithDetectionTask {
    pub fn new(
        trace_id: TraceId,
        request: GenerationWithDetectionHttpRequest,
        headers: HeaderMap,
    ) -> Self {
        Self {
            trace_id,
            model_id: request.model_id,
            prompt: request.prompt,
            detectors: request.detectors,
            text_gen_parameters: request.text_gen_parameters,
            headers,
        }
    }
}
