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
use std::{collections::HashMap, sync::Arc};

use futures::future::try_join_all;
use tracing::{Instrument, error, info, instrument};
use uuid::Uuid;

use super::CompletionsDetectionTask;
use crate::{
    clients::openai::*,
    config::DetectorType,
    models::{
        DetectionWarningReason, DetectorParams, UNSUITABLE_INPUT_MESSAGE, UNSUITABLE_OUTPUT_MESSAGE,
    },
    orchestrator::{
        Context, Error,
        common::{self, validate_detectors},
        types::ChatMessageIterator,
    },
};

pub async fn handle_unary(
    ctx: Arc<Context>,
    task: CompletionsDetectionTask,
) -> Result<CompletionsResponse, Error> {
    let trace_id = task.trace_id;
    let detectors = task.request.detectors.clone();
    info!(%trace_id, config = ?detectors, "task started");
    let input_detectors = detectors.input;
    let output_detectors = detectors.output;

    validate_detectors(
        &input_detectors,
        &ctx.config.detectors,
        &[DetectorType::TextContents],
        true,
    )?;

    validate_detectors(
        &output_detectors,
        &ctx.config.detectors,
        &[DetectorType::TextContents],
        true,
    )?;

    if !input_detectors.is_empty() {
        // Handle input detection
        match handle_input_detection(ctx.clone(), &task, input_detectors).await {
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

    // Handle chat completion
    let client = ctx
        .clients
        .get_as::<OpenAiClient>("chat_completions")
        .unwrap();
    let chat_completion =
        match common::chat_completion(client, task.headers.clone(), task.request.clone()).await {
            Ok(ChatCompletionsResponse::Unary(chat_completion)) => *chat_completion,
            Ok(ChatCompletionsResponse::Streaming(_)) => unimplemented!(),
            Err(error) => return Err(error),
        };

    if !output_detectors.is_empty() {
        // Handle output detection
        let chat_completion =
            handle_output_detection(ctx.clone(), task, output_detectors, chat_completion).await?;
        Ok(chat_completion.into())
    } else {
        // No output detectors, send chat completion response
        Ok(chat_completion.into())
    }
}

#[instrument(skip_all)]
async fn handle_input_detection(
    ctx: Arc<Context>,
    task: &CompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
) -> Result<Option<ChatCompletion>, Error> {
    let trace_id = task.trace_id;
    let model_id = task.request.model.clone();

    // Input detectors are only applied to the last message
    // If this changes, the empty content validation in [`ChatCompletionsRequest::validate`]
    // should also change.
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
    let detections = match common::text_contents_detections(
        ctx.clone(),
        task.headers.clone(),
        detectors.clone(),
        input_id,
        vec![(0, input_text)],
    )
    .await
    {
        Ok((_, detections)) => detections,
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
            created: common::current_timestamp().as_secs() as i64,
            detections: Some(ChatDetections {
                input: vec![InputDetectionResult {
                    message_index: message.index,
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

#[instrument(skip_all)]
async fn handle_output_detection(
    ctx: Arc<Context>,
    task: CompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
    mut chat_completion: ChatCompletion,
) -> Result<ChatCompletion, Error> {
    let mut tasks = Vec::with_capacity(chat_completion.choices.len());
    for choice in &chat_completion.choices {
        if choice
            .message
            .content
            .as_ref()
            .is_none_or(|content| content.is_empty())
        {
            chat_completion.warnings.push(OrchestratorWarning::new(
                DetectionWarningReason::EmptyOutput,
                &format!(
                    "Choice of index {} has no content. Output detection was not executed",
                    choice.index
                ),
            ));
            continue;
        }
        let input_id = choice.index;
        let input_text = choice.message.content.clone().unwrap_or_default();
        tasks.push(tokio::spawn(
            common::text_contents_detections(
                ctx.clone(),
                task.headers.clone(),
                detectors.clone(),
                input_id,
                vec![(0, input_text)],
            )
            .in_current_span(),
        ));
    }
    let detections = try_join_all(tasks)
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, Error>>()?;
    if !detections.is_empty() {
        // Update chat completion with detections
        let output = detections
            .into_iter()
            .filter(|(_, detections)| !detections.is_empty())
            .map(|(input_id, detections)| OutputDetectionResult {
                choice_index: input_id,
                results: detections.into(),
            })
            .collect::<Vec<_>>();
        if !output.is_empty() {
            chat_completion.detections = Some(ChatDetections {
                output,
                ..Default::default()
            });
            chat_completion.warnings = vec![OrchestratorWarning::new(
                DetectionWarningReason::UnsuitableOutput,
                UNSUITABLE_OUTPUT_MESSAGE,
            )];
        }
    }
    Ok(chat_completion)
}
