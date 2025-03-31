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
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    models::{DetectorParams, StreamingContentDetectionRequest, StreamingContentDetectionResponse},
    orchestrator::{Error, Orchestrator, types::BoxStream},
};

use super::Handle;

impl Handle<StreamingContentDetectionTask> for Orchestrator {
    type Response = ReceiverStream<Result<StreamingContentDetectionResponse, Error>>;

    async fn handle(&self, _task: StreamingContentDetectionTask) -> Result<Self::Response, Error> {
        todo!()
    }
}

pub struct StreamingContentDetectionTask {
    pub trace_id: TraceId,
    pub headers: HeaderMap,
    pub detectors: HashMap<String, DetectorParams>,
    pub input_stream: BoxStream<Result<StreamingContentDetectionRequest, Error>>,
}

impl StreamingContentDetectionTask {
    pub fn new(
        trace_id: TraceId,
        headers: HeaderMap,
        input_stream: BoxStream<Result<StreamingContentDetectionRequest, Error>>,
    ) -> Self {
        Self {
            trace_id,
            headers,
            detectors: HashMap::default(),
            input_stream,
        }
    }
}
