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
    clients::openai,
    config::DetectorType,
    models::{ChatDetectionHttpRequest, ChatDetectionResult, DetectorParams},
    orchestrator::{
        Error, Orchestrator,
        common::{self, validate_detectors},
    },
};

impl Handle<ChatDetectionTask> for Orchestrator {
    type Response = ChatDetectionResult;

    #[instrument(
        name = "chat_detection",
        skip_all,
        fields(trace_id = ?task.trace_id, headers = ?task.headers)
    )]
    async fn handle(&self, task: ChatDetectionTask) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        let trace_id = task.trace_id;
        info!(%trace_id, config = ?task.detectors, "task started");

        validate_detectors(
            &task.detectors,
            &ctx.config.detectors,
            &[DetectorType::TextChat],
            true,
        )?;

        // Handle detection
        let detections = common::text_chat_detections(
            ctx,
            task.headers,
            task.detectors,
            task.messages,
            task.tools,
        )
        .await?;

        Ok(ChatDetectionResult {
            detections: detections.into(),
        })
    }
}

#[derive(Debug)]
pub struct ChatDetectionTask {
    /// Trace ID
    pub trace_id: TraceId,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    /// Messages to run detection on
    pub messages: Vec<openai::Message>,
    /// Tools
    pub tools: Vec<openai::Tool>,
    /// Headers
    pub headers: HeaderMap,
}

impl ChatDetectionTask {
    pub fn new(trace_id: TraceId, request: ChatDetectionHttpRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
            detectors: request.detectors,
            messages: request.messages,
            tools: request.tools,
            headers,
        }
    }
}
