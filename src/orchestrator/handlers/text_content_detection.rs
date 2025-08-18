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
    config::DetectorType,
    models::{DetectorParams, TextContentDetectionHttpRequest, TextContentDetectionResult},
    orchestrator::{
        Error, Orchestrator,
        common::{self, validate_detectors},
    },
};

impl Handle<TextContentDetectionTask> for Orchestrator {
    type Response = TextContentDetectionResult;

    #[instrument(
        name = "text_content_detection",
        skip_all,
        fields(trace_id = ?task.trace_id, headers = ?task.headers)
    )]
    async fn handle(&self, task: TextContentDetectionTask) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        let trace_id = task.trace_id;
        info!(%trace_id, config = ?task.detectors, "task started");

        validate_detectors(
            &task.detectors,
            &ctx.config.detectors,
            &[DetectorType::TextContents],
            true,
        )?;

        // Handle detection
        let detections = common::text_contents_detections(
            ctx,
            task.headers,
            task.detectors,
            vec![(0, task.content)],
        )
        .await?;

        Ok(TextContentDetectionResult {
            detections: detections.into_iter().map(Into::into).collect(),
        })
    }
}

#[derive(Debug)]
pub struct TextContentDetectionTask {
    /// Trace ID
    pub trace_id: TraceId,
    /// Content text
    pub content: String,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    /// Headers
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
            content: request.content,
            detectors: request.detectors,
            headers,
        }
    }
}
