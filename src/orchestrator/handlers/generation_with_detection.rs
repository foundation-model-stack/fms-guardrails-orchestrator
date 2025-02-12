use std::collections::HashMap;

use http::HeaderMap;
use opentelemetry::trace::TraceId;

use super::Handle;
use crate::{
    models::{
        DetectorParams, GenerationWithDetectionHttpRequest, GenerationWithDetectionResult,
        GuardrailsTextGenerationParameters,
    },
    orchestrator::{Error, Orchestrator},
};

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
