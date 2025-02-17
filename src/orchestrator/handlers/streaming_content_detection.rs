use super::prelude::*;

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
