use http::HeaderMap;
use opentelemetry::trace::TraceId;

use super::Handle;
use crate::{
    clients::openai::{ChatCompletionsRequest, ChatCompletionsResponse},
    orchestrator::{Error, Orchestrator},
};

impl Handle<ChatCompletionsDetectionTask> for Orchestrator {
    type Response = ChatCompletionsResponse;

    async fn handle(&self, _task: ChatCompletionsDetectionTask) -> Result<Self::Response, Error> {
        // for streaming, build new ChatCompletionChunk from ChatCompletionDetectionBatch
        // and a cache of original ChatCompletionChunks
        todo!()
    }
}

#[derive(Debug)]
pub struct ChatCompletionsDetectionTask {
    /// Unique identifier of request trace
    pub trace_id: TraceId,
    /// Chat completion request
    pub request: ChatCompletionsRequest,
    // Headermap
    pub headers: HeaderMap,
}

impl ChatCompletionsDetectionTask {
    pub fn new(trace_id: TraceId, request: ChatCompletionsRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
            request,
            headers,
        }
    }
}
