use std::collections::HashMap;

use http::HeaderMap;
use opentelemetry::trace::TraceId;

use super::Handle;
use crate::{
    clients::openai,
    models::{ChatDetectionHttpRequest, ChatDetectionResult, DetectorParams},
    orchestrator::{Error, Orchestrator},
};

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
            detectors: request.detectors, // unnecessary copy
            messages: request.messages,   // unnecessary copy
            headers,
        }
    }
}
