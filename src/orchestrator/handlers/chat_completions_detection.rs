use super::prelude::*;
use crate::clients::openai::{ChatCompletionsRequest, ChatCompletionsResponse};

impl Handle<ChatCompletionsDetectionTask> for Orchestrator {
    type Response = ChatCompletionsResponse;

    async fn handle(&self, _task: ChatCompletionsDetectionTask) -> Result<Self::Response, Error> {
        todo!()
    }
}

async fn handle_unary(
    _ctx: Arc<Context>,
    _task: ChatCompletionsDetectionTask,
) -> Result<ChatCompletionsResponse, Error> {
    todo!()
}

async fn handle_streaming(
    _ctx: Arc<Context>,
    _task: ChatCompletionsDetectionTask,
) -> Result<ChatCompletionsResponse, Error> {
    todo!()
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
