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
        common::{self, group_detections_by_choice, group_detectors_by_type, validate_detectors},
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
        input_detectors.iter(),
        &ctx.config.detectors,
        &[DetectorType::TextContents],
        true,
    )?;
    validate_detectors(
        output_detectors.iter(),
        &ctx.config.detectors,
        &[DetectorType::TextContents, DetectorType::TextGeneration],
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

    // Handle completion
    let client = ctx.clients.get::<OpenAiClient>("openai").unwrap();
    let completion =
        match common::completion(client, task.headers.clone(), task.request.clone()).await {
            Ok(CompletionsResponse::Unary(completion)) => *completion,
            Ok(CompletionsResponse::Streaming(_)) => unimplemented!(),
            Err(error) => return Err(error),
        };

    if !output_detectors.is_empty() {
        // Handle output detection
        let completion =
            handle_output_detection(ctx.clone(), task, output_detectors, completion).await?;
        Ok(completion.into())
    } else {
        // No output detectors, send completion response
        Ok(completion.into())
    }
}

#[instrument(skip_all)]
async fn handle_input_detection(
    ctx: Arc<Context>,
    task: &CompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
) -> Result<Option<Completion>, Error> {
    let trace_id = task.trace_id;
    let model_id = task.request.model.clone();
    let inputs = common::apply_masks(
        task.request.prompt.clone(),
        task.request.prompt_masks.as_deref(),
    );
    let detections = match common::text_contents_detections(
        ctx.clone(),
        task.headers.clone(),
        detectors.clone(),
        inputs,
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
            prompt: Some(task.request.prompt.clone()),
            ..Default::default()
        };
        let tokenize_response =
            common::tokenize_openai(client, task.headers.clone(), tokenize_request).await?;
        let usage = Usage {
            prompt_tokens: tokenize_response.count,
            ..Default::default()
        };

        // Build completion with input detections
        let completion = Completion {
            id: Uuid::new_v4().simple().to_string(),
            created: common::current_timestamp().as_secs() as i64,
            model: model_id,
            detections: Some(CompletionDetections {
                input: vec![CompletionInputDetections {
                    message_index: 0,
                    results: detections,
                }],
                ..Default::default()
            }),
            warnings: vec![CompletionDetectionWarning::new(
                DetectionWarningReason::UnsuitableInput,
                UNSUITABLE_INPUT_MESSAGE,
            )],
            usage: Some(usage),
            ..Default::default()
        };
        Ok(Some(completion))
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
    mut completion: Completion,
) -> Result<Completion, Error> {
    use DetectorType::*;
    let detector_groups = group_detectors_by_type(&ctx, detectors);
    let headers = &task.headers;
    let prompt = &task.request.prompt;

    // Spawn detection tasks
    let mut tasks = Vec::with_capacity(completion.choices.len() * detector_groups.len());
    for choice in &completion.choices {
        if choice.text.is_empty() {
            // Add no content warning
            completion.warnings.push(CompletionDetectionWarning::new(
                DetectionWarningReason::EmptyOutput,
                &format!(
                    "Choice of index {} has no content. Output detection was not executed",
                    choice.index
                ),
            ));
            continue;
        }
        for (detector_type, detectors) in &detector_groups {
            let detection_task = match detector_type {
                TextContents => tokio::spawn(
                    common::text_contents_detections(
                        ctx.clone(),
                        headers.clone(),
                        detectors.clone(),
                        vec![(0, choice.text.clone())],
                    )
                    .in_current_span(),
                ),
                TextGeneration => tokio::spawn(
                    common::text_generation_detections(
                        ctx.clone(),
                        headers.clone(),
                        detectors.clone(),
                        prompt.clone(),
                        choice.text.clone(),
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
            completion.warnings.push(CompletionDetectionWarning::new(
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
            completion.detections = Some(CompletionDetections {
                output,
                ..Default::default()
            });
        }
    }
    Ok(completion)
}
