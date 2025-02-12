use http::HeaderMap;
use opentelemetry::trace::TraceId;
use tokio_stream::wrappers::ReceiverStream;

use super::Handle;
use crate::{
    models::{
        ClassifiedGeneratedTextStreamResult, GuardrailsConfig, GuardrailsHttpRequest,
        GuardrailsTextGenerationParameters,
    },
    orchestrator::{Error, Orchestrator},
};

impl Handle<StreamingClassificationWithGenTask> for Orchestrator {
    type Response = ReceiverStream<Result<ClassifiedGeneratedTextStreamResult, Error>>;

    async fn handle(
        &self,
        _task: StreamingClassificationWithGenTask,
    ) -> Result<Self::Response, Error> {
        todo!()
    }
}

#[derive(Debug)]
pub struct StreamingClassificationWithGenTask {
    pub trace_id: TraceId,
    pub model_id: String,
    pub inputs: String,
    pub guardrails_config: GuardrailsConfig,
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
    pub headers: HeaderMap,
}

impl StreamingClassificationWithGenTask {
    pub fn new(trace_id: TraceId, request: GuardrailsHttpRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
            model_id: request.model_id,
            inputs: request.inputs,
            guardrails_config: request.guardrail_config.unwrap_or_default(),
            text_gen_parameters: request.text_gen_parameters,
            headers,
        }
    }
}
