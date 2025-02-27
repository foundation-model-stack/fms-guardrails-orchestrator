use futures::future::try_join_all;
use uuid::Uuid;

use super::super::prelude::*;
use crate::clients::openai::*;

pub async fn handle_unary(
    ctx: Arc<Context>,
    task: ChatCompletionsDetectionTask,
) -> Result<ChatCompletionsResponse, Error> {
    let trace_id = &task.trace_id;
    let detector_config = &task.request.detectors;
    let input_detectors = detector_config
        .as_ref()
        .map(|config| config.input.clone().unwrap_or_default());
    let output_detectors = detector_config
        .as_ref()
        .map(|config| config.output.clone().unwrap_or_default());
    info!(%trace_id, "task started");

    // TODO: validate guardrails

    if let Some(detectors) = input_detectors {
        // Handle input detection
        match handle_input_detection(ctx.clone(), &task, &detectors).await {
            Ok(Some(completion)) => {
                info!(%trace_id, "task completed: returning response with input detections");
                // Return response with input detections and terminate
                let response = completion.into();
                return Ok(response);
            }
            Ok(None) => (), // No input detections
            Err(error) => {
                // Input detections failed
                return Err(error);
            }
        }
    }

    // Get chat completion
    let chat_completion = match common::chat_completion(
        ctx.clone(),
        task.headers.clone(),
        task.request.clone(),
    )
    .await
    {
        Ok(ChatCompletionsResponse::Unary(chat_completion)) => *chat_completion,
        Ok(ChatCompletionsResponse::Streaming(_)) => unimplemented!(),
        Err(error) => return Err(error),
    };

    if let Some(detectors) = output_detectors {
        // Handle output detection
        let chat_completion =
            handle_output_detection(ctx.clone(), &task, detectors.clone(), chat_completion).await?;
        Ok(chat_completion.into())
    } else {
        // No output detectors, send chat completion response
        Ok(chat_completion.into())
    }
}

async fn handle_input_detection(
    ctx: Arc<Context>,
    task: &ChatCompletionsDetectionTask,
    detectors: &HashMap<String, DetectorParams>,
) -> Result<Option<ChatCompletion>, Error> {
    let trace_id = &task.trace_id;
    let headers = &task.headers;
    let model_id = task.request.model.clone();

    // Input detectors are only applied to the last message
    // Get the last message
    let messages = task.request.messages();
    let message = if let Some(message) = messages.last() {
        message
    } else {
        return Err(Error::Validation("No messages provided".into()));
    };
    // Validate role
    if !matches!(
        message.role,
        Some(Role::User) | Some(Role::Assistant) | Some(Role::System)
    ) {
        return Err(Error::Validation(
            "Last message role must be user, assistant, or system".into(),
        ));
    }
    let input_id = message.index;
    let input_text = message.text.map(|s| s.to_string()).unwrap_or_default();
    let inputs = vec![(0, input_text)];

    let detections = match common::text_contents_detections(
        ctx.clone(),
        headers.clone(),
        detectors.clone(),
        input_id,
        inputs,
    )
    .await
    {
        Ok((input_id, detections)) => detections,
        Err(error) => {
            error!(%trace_id, %error, "task failed: error processing input detections");
            return Err(error);
        }
    };
    if !detections.is_empty() {
        // Build chat completion with input detections
        let chat_completion = ChatCompletion {
            id: Uuid::new_v4().simple().to_string(),
            model: model_id,
            created: common::current_timestamp_secs(),
            detections: Some(ChatDetections {
                input: vec![InputDetectionResult {
                    message_index: 0,
                    results: detections.into(),
                }],
                ..Default::default()
            }),
            warnings: vec![OrchestratorWarning::new(
                DetectionWarningReason::UnsuitableInput,
                UNSUITABLE_INPUT_MESSAGE,
            )],
            ..Default::default()
        };
        Ok(Some(chat_completion))
    } else {
        // No input detections
        Ok(None)
    }
}

async fn handle_output_detection(
    ctx: Arc<Context>,
    task: &ChatCompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
    mut chat_completion: ChatCompletion,
) -> Result<ChatCompletion, Error> {
    let headers = &task.headers;
    let mut tasks = Vec::with_capacity(chat_completion.choices.len());
    for choice in &chat_completion.choices {
        let detectors = detectors.clone();
        let input_id = choice.index;
        let input_text = choice.message.content.clone().unwrap_or_default();
        let inputs = vec![(0, input_text)];
        tasks.push(tokio::spawn(common::text_contents_detections(
            ctx.clone(),
            headers.clone(),
            detectors,
            input_id,
            inputs,
        )));
    }
    let detections = try_join_all(tasks)
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, Error>>()?;
    if !detections.is_empty() {
        // Update chat completion with detections
        let output = detections
            .into_iter()
            .map(|(input_id, detections)| OutputDetectionResult {
                choice_index: input_id,
                results: detections.into(),
            })
            .collect::<Vec<_>>();
        chat_completion.detections = Some(ChatDetections {
            output,
            ..Default::default()
        });
        chat_completion.warnings = vec![OrchestratorWarning::new(
            DetectionWarningReason::UnsuitableOutput,
            UNSUITABLE_OUTPUT_MESSAGE,
        )];
    }

    Ok(chat_completion)
}
