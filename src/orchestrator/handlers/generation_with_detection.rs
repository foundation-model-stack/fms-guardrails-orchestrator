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
use tracing::{info, instrument};

use super::Handle;
use crate::{
    clients::GenerationClient,
    config::DetectorType,
    models::{
        DetectorParams, GenerationWithDetectionHttpRequest, GenerationWithDetectionResult,
        GuardrailsTextGenerationParameters,
    },
    orchestrator::{
        Error, Orchestrator,
        common::{self, validate_detectors},
    },
};

impl Handle<GenerationWithDetectionTask> for Orchestrator {
    type Response = GenerationWithDetectionResult;

    #[instrument(
        name = "generation_with_detection",
        skip_all,
        fields(trace_id = ?task.trace_id, headers = ?task.headers)
    )]
    async fn handle(&self, task: GenerationWithDetectionTask) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        let trace_id = task.trace_id;
        info!(%trace_id, config = ?task.detectors, "task started");

        validate_detectors(
            &task.detectors,
            &ctx.config.detectors,
            &[DetectorType::TextGeneration],
            true,
        )?;

        // Handle generation
        let client = ctx.clients.get::<GenerationClient>("generation").unwrap();
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
    /// Trace ID
    pub trace_id: TraceId,
    /// Model ID
    pub model_id: String,
    /// Prompt text
    pub prompt: String,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    /// Text generation parameters
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
    /// Headers
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
