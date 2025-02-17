use super::prelude::*;

impl Handle<ClassificationWithGenTask> for Orchestrator {
    type Response = ClassifiedGeneratedTextResult;

    async fn handle(&self, task: ClassificationWithGenTask) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        let headers = task.headers;
        let guardrails = task.guardrails_config;
        let input_detectors = guardrails.input_detectors();
        let output_detectors = guardrails.output_detectors();
        let input_text = task.inputs;

        // Validate guardrails config
        // validate_guardrails(&ctx, &guardrails)?;

        // Process input detections
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

        if let Some(detections) = input_detections {
            // let detections = detections?;
            // detections.sort_by_key(|r| r.start);

            // Get token count
            let (input_token_count, _) = common::tokenize(
                ctx.clone(),
                headers.clone(),
                task.model_id.clone(),
                input_text.clone(),
            )
            .await?;

            // Send response with input detections
            let response = ClassifiedGeneratedTextResult {
                input_token_count,
                warnings: Some(vec![DetectionWarning::unsuitable_input()]),
                ..Default::default()
            };
            // response.token_classification_results.input = Some(detections);
            Ok(response)
        } else {
            // Process generation
            let generation = common::generate(
                ctx.clone(),
                headers.clone(),
                task.model_id,
                input_text.clone(),
                task.text_gen_parameters,
            )
            .await?;

            // Process output detections
            let output_detections = output_detectors
                .async_map(|detectors| async {
                    let generated_text = generation.generated_text.clone().unwrap_or_default();
                    let inputs = vec![(0, generated_text)];
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

            // Send response with output detections
            // let mut response = generation;
            // if let Some(detections) = output_detections {
            //     // response.token_classification_results.output = Some(detections);
            //     // response.warnings = Some(vec![DetectionWarning::unsuitable_output()]);
            //     todo!()
            // }
            // Ok(response)
            todo!()
        }
    }
}

#[derive(Debug)]
pub struct ClassificationWithGenTask {
    pub trace_id: TraceId,
    pub model_id: String,
    pub inputs: String,
    pub guardrails_config: GuardrailsConfig,
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
    pub headers: HeaderMap,
}

impl ClassificationWithGenTask {
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
