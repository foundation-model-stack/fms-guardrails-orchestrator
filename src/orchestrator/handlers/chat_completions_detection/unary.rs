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

use super::ChatCompletionsDetectionTask;
use crate::{
    clients::openai::*,
    config::DetectorType,
    models::{
        DetectionWarningReason, DetectorParams, UNSUITABLE_INPUT_MESSAGE, UNSUITABLE_OUTPUT_MESSAGE,
    },
    orchestrator::{
        Context, Error,
        common::{self, group_detections_by_choice, group_detectors_by_type, validate_detectors},
        types::ChatMessageIterator,
    },
};

pub async fn handle_unary(
    ctx: Arc<Context>,
    task: ChatCompletionsDetectionTask,
) -> Result<ChatCompletionsResponse, Error> {
    let trace_id = task.trace_id;
    let detectors = task.request.detectors.clone();
    info!(%trace_id, config = ?detectors, "task started");
    let input_detectors = detectors.input;
    let output_detectors = detectors.output;

    validate_detectors(
        input_detectors.iter(),
        &ctx.config.detectors,
        &[DetectorType::TextContents],
        true,
    )?;
    validate_detectors(
        output_detectors.iter(),
        &ctx.config.detectors,
        &[DetectorType::TextContents, DetectorType::TextChat],
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
    let client = ctx.clients.get::<OpenAiClient>("openai").unwrap();
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
    task: &ChatCompletionsDetectionTask,
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
    let input_text = message.text.map(|s| s.to_string()).unwrap_or_default();
    let detections = match common::text_contents_detections(
        ctx.clone(),
        task.headers.clone(),
        detectors.clone(),
        vec![(0, input_text.clone())],
    )
    .await
    {
        Ok(detections) => detections,
        Err(error) => {
            error!(%trace_id, %error, "task failed: error processing input detections");
            return Err(error);
        }
    };
    if !detections.is_empty() {
        // Get prompt tokens for usage
        let client = ctx.clients.get::<OpenAiClient>("openai").unwrap();
        let tokenize_request = TokenizeRequest {
            model: model_id.clone(),
            prompt: Some(input_text),
            ..Default::default()
        };
        let tokenize_response =
            common::tokenize_openai(client, task.headers.clone(), tokenize_request).await?;
        let usage = Usage {
            prompt_tokens: tokenize_response.count,
            ..Default::default()
        };

        // Build chat completion with input detections
        let chat_completion = ChatCompletion {
            id: Uuid::new_v4().simple().to_string(),
            model: model_id,
            created: common::current_timestamp().as_secs() as i64,
            detections: Some(CompletionDetections {
                input: vec![CompletionInputDetections {
                    message_index: message.index,
                    results: detections,
                }],
                ..Default::default()
            }),
            warnings: vec![CompletionDetectionWarning::new(
                DetectionWarningReason::UnsuitableInput,
                UNSUITABLE_INPUT_MESSAGE,
            )],
            usage,
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
    task: ChatCompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
    mut chat_completion: ChatCompletion,
) -> Result<ChatCompletion, Error> {
    use DetectorType::*;
    let detector_groups = group_detectors_by_type(&ctx, detectors);
    let headers = &task.headers;
    let messages = task.request.messages.as_slice();
    let tools = task.request.tools.as_ref().cloned().unwrap_or_default();

    // Spawn detection tasks
    let mut tasks = Vec::with_capacity(chat_completion.choices.len() * detector_groups.len());
    for choice in &chat_completion.choices {
        if !choice.message.has_content() {
            // Add no content warning
            chat_completion
                .warnings
                .push(CompletionDetectionWarning::new(
                    DetectionWarningReason::EmptyOutput,
                    &format!("Choice of index {} has no content", choice.index),
                ));
        }
        for (detector_type, detectors) in &detector_groups {
            let detection_task = match detector_type {
                TextContents => match choice.message.text() {
                    Some(content_text) => tokio::spawn(
                        common::text_contents_detections(
                            ctx.clone(),
                            headers.clone(),
                            detectors.clone(),
                            vec![(0, content_text.clone())],
                        )
                        .in_current_span(),
                    ),
                    _ => continue, // no content, skip
                },
                TextChat => tokio::spawn(
                    common::text_chat_detections(
                        ctx.clone(),
                        headers.clone(),
                        detectors.clone(),
                        [messages, std::slice::from_ref(&choice.message)].concat(),
                        tools.clone(),
                    )
                    .in_current_span(),
                ),
                _ => unimplemented!(),
            };
            tasks.push((choice.index, *detector_type, detection_task));
        }
    }

    // Await completion of all detection tasks
    let detections = try_join_all(tasks.into_iter().map(
        |(choice_index, detector_type, detection_task)| async move {
            Ok::<_, Error>((choice_index, detector_type, detection_task.await?))
        },
    ))
    .await?
    .into_iter()
    .map(|(choice_index, detector_type, result)| {
        result.map(|detections| (choice_index, detector_type, detections))
    })
    .collect::<Result<Vec<_>, Error>>()?;

    if !detections.is_empty() {
        // If there are text contents detections, add unsuitable output warning
        let unsuitable_output = detections.iter().any(|(_, detector_type, detections)| {
            matches!(detector_type, TextContents) && !detections.is_empty()
        });
        if unsuitable_output {
            chat_completion
                .warnings
                .push(CompletionDetectionWarning::new(
                    DetectionWarningReason::UnsuitableOutput,
                    UNSUITABLE_OUTPUT_MESSAGE,
                ));
        }

        // Group detections by choice
        let detections_by_choice = group_detections_by_choice(detections);
        // Update completion with detections
        let output = detections_by_choice
            .into_iter()
            .map(|(choice_index, detections)| CompletionOutputDetections {
                choice_index,
                results: detections,
            })
            .collect::<Vec<_>>();
        if !output.is_empty() {
            chat_completion.detections = Some(CompletionDetections {
                output,
                ..Default::default()
            });
        }
    }
    Ok(chat_completion)
}
