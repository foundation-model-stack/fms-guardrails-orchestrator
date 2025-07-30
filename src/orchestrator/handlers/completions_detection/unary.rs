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
        input_detectors.iter().chain(output_detectors.iter()),
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
        0,
        inputs,
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
            object: "text_completion".into(), // This value is constant: https://platform.openai.com/docs/api-reference/completions/object#completions/object-object
            created: common::current_timestamp().as_secs() as i64,
            model: model_id,
            detections: Some(CompletionDetections {
                input: vec![CompletionInputDetections {
                    message_index: 0,
                    results: detections.into(),
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
    let mut tasks = Vec::with_capacity(completion.choices.len());
    for choice in &completion.choices {
        if choice.text.is_empty() {
            completion.warnings.push(CompletionDetectionWarning::new(
                DetectionWarningReason::EmptyOutput,
                &format!(
                    "Choice of index {} has no content. Output detection was not executed",
                    choice.index
                ),
            ));
            continue;
        }
        let input_id = choice.index;
        let input_text = choice.text.clone();
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
        // Update completion with detections
        let output = detections
            .into_iter()
            .filter(|(_, detections)| !detections.is_empty())
            .map(|(input_id, detections)| CompletionOutputDetections {
                choice_index: input_id,
                results: detections.into(),
            })
            .collect::<Vec<_>>();
        if !output.is_empty() {
            completion.detections = Some(CompletionDetections {
                output,
                ..Default::default()
            });
            completion.warnings = vec![CompletionDetectionWarning::new(
                DetectionWarningReason::UnsuitableOutput,
                UNSUITABLE_OUTPUT_MESSAGE,
            )];
        }
    }
    Ok(completion)
}
