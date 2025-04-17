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
    clients::detector::ContextType,
    config::DetectorType,
    models::{ContextDocsHttpRequest, ContextDocsResult, DetectorParams},
    orchestrator::{
        Error, Orchestrator,
        common::{self, validate_detectors},
    },
};

impl Handle<ContextDocsDetectionTask> for Orchestrator {
    type Response = ContextDocsResult;

    #[instrument(
        name = "context_docs_detection",
        skip_all,
        fields(trace_id = ?task.trace_id, headers = ?task.headers)
    )]
    async fn handle(&self, task: ContextDocsDetectionTask) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        let trace_id = task.trace_id;
        info!(%trace_id, config = ?task.detectors, "task started");

        validate_detectors(
            &task.detectors,
            &ctx.config.detectors,
            &[DetectorType::TextContextDoc],
            true,
        )?;

        // Handle detection
        let detections = common::text_context_detections(
            ctx,
            task.headers,
            task.detectors,
            task.content,
            task.context_type,
            task.context,
        )
        .await?;

        Ok(ContextDocsResult {
            detections: detections.into(),
        })
    }
}

#[derive(Debug)]
pub struct ContextDocsDetectionTask {
    /// Trace ID
    pub trace_id: TraceId,
    /// Content text
    pub content: String,
    /// Context type
    pub context_type: ContextType,
    /// Context
    pub context: Vec<String>,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    /// Headers
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
