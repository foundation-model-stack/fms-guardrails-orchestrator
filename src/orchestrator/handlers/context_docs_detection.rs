use std::collections::HashMap;

use http::HeaderMap;
use opentelemetry::trace::TraceId;

use super::Handle;
use crate::{
    clients::detector::ContextType,
    models::{ContextDocsHttpRequest, ContextDocsResult, DetectorParams},
    orchestrator::{Error, Orchestrator},
};

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
