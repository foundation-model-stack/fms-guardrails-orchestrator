use super::prelude::*;

impl Handle<StreamingClassificationWithGenTask> for Orchestrator {
    type Response = ReceiverStream<Result<ClassifiedGeneratedTextStreamResult, Error>>;

    async fn handle(
        &self,
        task: StreamingClassificationWithGenTask,
    ) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        let headers = task.headers;
        let guardrails = task.guardrails_config;
        let input_detectors = guardrails.input_detectors();
        let output_detectors = guardrails.output_detectors();
        let input_text = task.inputs;

        let (response_tx, response_rx) =
            mpsc::channel::<Result<ClassifiedGeneratedTextStreamResult, Error>>(32);

        // Validate guardrails config
        // validate_guardrails(&ctx, &guardrails)?;

        // Process input detections (unary)
        let input_detections = input_detectors
            .async_map(|detectors| async {
                let inputs = common::apply_masks(input_text.clone(), guardrails.input_masks());
                let detections = common::text_contents_detections(
                    ctx.clone(),
                    headers.clone(),
                    detectors,
                    inputs,
                )
                .await?;
                Ok::<_, Error>(detections)
            })
            .await;

        // if let Some(input_detections) = input_detections {
        //     // Get token count
        //     // Send response with input detections
        //     todo!()
        // } else {
        //     // Process output detections (streaming)
        //     todo!()
        // }

        Ok(ReceiverStream::new(response_rx))
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
