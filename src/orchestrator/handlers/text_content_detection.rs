use std::collections::HashMap;

use http::HeaderMap;
use opentelemetry::trace::TraceId;

use super::Handle;
use crate::{
    models::{DetectorParams, TextContentDetectionHttpRequest, TextContentDetectionResult},
    orchestrator::{Error, Orchestrator},
};

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
            content: request.content,     // unnecessary copy
            detectors: request.detectors, // unnecessary copy
            headers,
        }
    }
}
