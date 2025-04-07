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
use tracing::info;

use super::Handle;
use crate::{
    clients::GenerationClient,
    models::{
        DetectorParams, GenerationWithDetectionHttpRequest, GenerationWithDetectionResult,
        GuardrailsTextGenerationParameters,
    },
    orchestrator::{Error, Orchestrator, common},
};

impl Handle<GenerationWithDetectionTask> for Orchestrator {
    type Response = GenerationWithDetectionResult;

    async fn handle(&self, task: GenerationWithDetectionTask) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        let trace_id = task.trace_id;
        info!(%trace_id, "task started");

        // TODO: validate requested guardrails

        // Handle generation
        let client = ctx
            .clients
            .get_as::<GenerationClient>("generation")
            .unwrap();
        let generation = common::generate(
            client,
            task.headers.clone(),
            task.model_id.clone(),
            task.prompt.clone(),
            task.text_gen_parameters.clone(),
        )
        .await?;
        let generated_text = generation.generated_text.unwrap_or_default();

        // Handle detection
        let detections = common::text_generation_detections(
            ctx,
            task.headers,
            task.detectors,
            task.prompt,
            generated_text.clone(),
        )
        .await?;

        Ok(GenerationWithDetectionResult {
            generated_text,
            input_token_count: generation.input_token_count,
            detections: detections.into(),
        })
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
