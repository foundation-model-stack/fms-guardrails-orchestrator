use super::prelude::*;

impl Handle<ClassificationWithGenTask> for Orchestrator {
    type Response = ClassifiedGeneratedTextResult;

    async fn handle(&self, task: ClassificationWithGenTask) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        let trace_id = &task.trace_id;
        let guardrails = &task.guardrails_config;
        let input_detectors = guardrails.input_detectors();
        let output_detectors = guardrails.output_detectors();
        info!(%trace_id, "task started");

        // TODO: validate guardrails

        if let Some(detectors) = input_detectors {
            // Handle input detection
            match handle_input_detection(ctx.clone(), &task, detectors).await {
                Ok(Some(response)) => {
                    info!(%trace_id, "task completed: returning response with input detections");
                    // Return response with input detections and terminate
                    return Ok(response);
                }
                Ok(None) => (), // No input detections
                Err(error) => {
                    // Input detections failed
                    return Err(error);
                }
            }
        }

        // Handle generation
        let generation = common::generate(
            ctx.clone(),
            task.headers.clone(),
            task.model_id.clone(),
            task.inputs.clone(),
            task.text_gen_parameters.clone(),
        )
        .await?;

        if let Some(detectors) = output_detectors {
            // Handle output detection
            handle_output_detection(ctx.clone(), &task, detectors, generation).await
        } else {
            // No output detectors, return generation
            info!(%trace_id, "task completed: returning generation response");
            Ok(generation)
        }
    }
}

async fn handle_input_detection(
    ctx: Arc<Context>,
    task: &ClassificationWithGenTask,
    detectors: &HashMap<String, DetectorParams>,
) -> Result<Option<ClassifiedGeneratedTextResult>, Error> {
    let trace_id = &task.trace_id;
    let headers = &task.headers;
    let guardrails = &task.guardrails_config;
    let model_id = task.model_id.clone();
    let input_text = task.inputs.clone();

    let input_id = 0;
    let inputs = common::apply_masks(input_text.clone(), guardrails.input_masks());
    let detections = match common::text_contents_detections(
        ctx.clone(),
        headers.clone(),
        detectors.clone(),
        input_id,
        inputs,
    )
    .await
    {
        Ok(detections) => detections
            .into_iter()
            .flat_map(|(_input_id, _detector_id, detections)| detections)
            .collect::<Detections>(),
        Err(error) => {
            error!(%trace_id, %error, "task failed: error processing input detections");
            return Err(error);
        }
    };
    if !detections.is_empty() {
        // Get token count
        let input_token_count =
            match common::tokenize(ctx.clone(), headers.clone(), model_id, input_text).await {
                Ok((token_count, _tokens)) => token_count,
                Err(error) => {
                    error!(%trace_id, %error, "task failed: error tokenizing input text");
                    return Err(error);
                }
            };
        // Build response with input detections
        let response = input_detection_response(input_token_count, detections);
        Ok(Some(response))
    } else {
        // No input detections
        Ok(None)
    }
}

async fn handle_output_detection(
    ctx: Arc<Context>,
    task: &ClassificationWithGenTask,
    detectors: &HashMap<String, DetectorParams>,
    generation: ClassifiedGeneratedTextResult,
) -> Result<ClassifiedGeneratedTextResult, Error> {
    let trace_id = &task.trace_id;
    let headers = &task.headers;
    let generated_text = generation.generated_text.clone().unwrap_or_default();
    let input_id = 0;
    let inputs = vec![(0, generated_text)];
    let detections = match common::text_contents_detections(
        ctx,
        headers.clone(),
        detectors.clone(),
        input_id,
        inputs,
    )
    .await
    {
        Ok(detections) => detections
            .into_iter()
            .flat_map(|(_input_id, _detector_id, detections)| detections)
            .collect::<Detections>(),
        Err(error) => {
            error!(%trace_id, %error, "task failed: error processing input detections");
            return Err(error);
        }
    };
    let mut response = generation;
    if !detections.is_empty() {
        response.token_classification_results.output = Some(detections.into());
        response.warnings = Some(vec![DetectionWarning::unsuitable_output()]);
    }
    info!(%trace_id, "task completed: returning response with output detections");
    Ok(response)
}

/// Builds a response with input detections.
fn input_detection_response(
    input_token_count: u32,
    detections: Detections,
) -> ClassifiedGeneratedTextResult {
    ClassifiedGeneratedTextResult {
        input_token_count,
        token_classification_results: TextGenTokenClassificationResults {
            input: Some(detections.into()),
            output: None,
        },
        warnings: Some(vec![DetectionWarning::unsuitable_input()]),
        ..Default::default()
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
