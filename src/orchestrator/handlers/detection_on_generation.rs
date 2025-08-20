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
    models::{DetectionOnGeneratedHttpRequest, DetectionOnGenerationResult, DetectorParams},
    orchestrator::{
        Error, Orchestrator,
        common::{self, validate_detectors},
    },
};

impl Handle<DetectionOnGenerationTask> for Orchestrator {
    type Response = DetectionOnGenerationResult;

    #[instrument(
        name = "detection_on_generation",
        skip_all,
        fields(trace_id = ?task.trace_id, headers = ?task.headers)
    )]
    async fn handle(&self, task: DetectionOnGenerationTask) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        let trace_id = task.trace_id;
        info!(%trace_id, config = ?task.detectors, "task started");

        validate_detectors(
            &task.detectors,
            &ctx.config.detectors,
            &[DetectorType::TextGeneration],
            true,
        )?;

        // Handle detection
        let detections = common::text_generation_detections(
            ctx,
            task.headers,
            task.detectors,
            task.prompt,
            task.generated_text,
        )
        .await?;

        Ok(DetectionOnGenerationResult {
            detections: detections.into_iter().map(Into::into).collect(),
        })
    }
}

#[derive(Debug)]
pub struct DetectionOnGenerationTask {
    /// Trace ID
    pub trace_id: TraceId,
    /// Prompt text
    pub prompt: String,
    /// Generated text
    pub generated_text: String,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    /// Headers
    pub headers: HeaderMap,
}

impl DetectionOnGenerationTask {
    pub fn new(
        trace_id: TraceId,
        request: DetectionOnGeneratedHttpRequest,
        headers: HeaderMap,
    ) -> Self {
        Self {
            trace_id,
            prompt: request.prompt,
            generated_text: request.generated_text,
            detectors: request.detectors,
            headers,
        }
    }
}
