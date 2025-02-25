use uuid::Uuid;

use super::super::prelude::*;
use crate::clients::openai::*;

pub async fn handle_unary(
    ctx: Arc<Context>,
    task: ChatCompletionsDetectionTask,
) -> Result<ChatCompletionsResponse, Error> {
    let trace_id = &task.trace_id;
    let guardrails = &task.request.detectors;
    let input_detectors = guardrails
        .as_ref()
        .map(|config| config.input.clone().unwrap_or_default());
    let output_detectors = guardrails
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

    todo!()
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
    let input_text = message.text.map(|s| s.to_string()).unwrap_or_default();
    let inputs = vec![(0, input_text)];

    let detections =
        match common::text_contents_detections(ctx.clone(), headers.clone(), detectors, inputs)
            .await
        {
            Ok(detections) => detections
                .into_iter()
                .flat_map(|(_detector_id, detections)| detections)
                .collect::<Detections>(),
            Err(error) => {
                error!(%trace_id, %error, "task failed: error processing input detections");
                return Err(error);
            }
        };
    if !detections.is_empty() {
        // Build completion with input detections
        let completion = input_detection_response(model_id, detections);
        Ok(Some(completion))
    } else {
        // No input detections
        Ok(None)
    }
}

/// Builds a response with input detections.
fn input_detection_response(model_id: String, detections: Detections) -> ChatCompletion {
    ChatCompletion {
        id: Uuid::new_v4().simple().to_string(),
        model: model_id,
        created: common::current_timestamp_secs(),
        detections: Some(ChatDetections {
            input: vec![InputDetectionResult {
                message_index: 0,
                results: detections.into(),
            }],
            output: vec![],
        }),
        warnings: vec![OrchestratorWarning::new(
            DetectionWarningReason::UnsuitableInput,
            UNSUITABLE_INPUT_MESSAGE,
        )],
        ..Default::default()
    }
}
