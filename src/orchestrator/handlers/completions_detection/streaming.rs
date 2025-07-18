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

use futures::{StreamExt, TryStreamExt, stream};
use opentelemetry::trace::TraceId;
use tokio::sync::mpsc;
use tracing::{Instrument, debug, error, info, instrument, warn};
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
        common::{self, text_contents_detections, validate_detectors},
        types::{
            Chunk, CompletionBatcher, CompletionState, CompletionStream, DetectionBatchStream,
            Detections,
        },
    },
};

pub async fn handle_streaming(
    ctx: Arc<Context>,
    task: CompletionsDetectionTask,
) -> Result<CompletionsResponse, Error> {
    let trace_id = task.trace_id;
    let detectors = task.request.detectors.clone();
    info!(%trace_id, config = ?detectors, "task started");

    // Create response channel
    let (response_tx, response_rx) = mpsc::channel::<Result<Option<Completion>, Error>>(128);

    tokio::spawn(
        async move {
            let input_detectors = detectors.input;
            let output_detectors = detectors.output;

            // Validate input detectors
            if let Err(error) = validate_detectors(
                &input_detectors,
                &ctx.config.detectors,
                &[DetectorType::TextContents],
                true,
            ) {
                let _ = response_tx.send(Err(error)).await;
                // Send None to signal completion
                let _ = response_tx.send(Ok(None)).await;
                return;
            }
            // Validate output detectors
            if let Err(error) = validate_detectors(
                &output_detectors,
                &ctx.config.detectors,
                &[DetectorType::TextContents],
                true,
            ) {
                let _ = response_tx.send(Err(error)).await;
                // Send None to signal completion
                let _ = response_tx.send(Ok(None)).await;
                return;
            }

            // Handle input detection (unary)
            if !input_detectors.is_empty() {
                match handle_input_detection(ctx.clone(), &task, input_detectors).await {
                    Ok(Some(chunk)) => {
                        info!(%trace_id, "task completed: returning response with input detections");
                        // Send message with input detections to response channel and terminate
                        let _ = response_tx.send(Ok(Some(chunk))).await;
                        // Send None to signal completion
                        let _ = response_tx.send(Ok(None)).await;
                        return;
                    }
                    Ok(None) => (), // No input detections
                    Err(error) => {
                        // Input detections failed
                        // Send error to response channel and terminate
                        let _ = response_tx.send(Err(error)).await;
                        // Send None to signal completion
                        let _ = response_tx.send(Ok(None)).await;
                        return;
                    }
                }
            }

            // Create completions stream
            let client = ctx
                .clients
                .get_as::<OpenAiClient>("openai")
                .unwrap();
            let completion_stream = match common::completion_stream(client, task.headers.clone(), task.request.clone()).await {
                Ok(stream) => stream,
                Err(error) => {
                    error!(%trace_id, %error, "task failed: error creating completions stream");
                    // Send error to response channel and terminate
                    let _ = response_tx.send(Err(error)).await;
                    // Send None to signal completion
                    let _ = response_tx.send(Ok(None)).await;
                    return;
                }
            };

            if output_detectors.is_empty() {
                // No output detectors, forward completion chunks to response channel
                process_completion_stream(trace_id, completion_stream, None, None, Some(response_tx.clone())).await;
                info!(%trace_id, "task completed: completion stream closed");
            } else {
                // Handle output detection
                handle_output_detection(
                   ctx.clone(),
                    &task,
                    output_detectors,
                   completion_stream,
                    response_tx.clone(),
                )
                .await;
            }

            // Send None to signal completion
            let _ = response_tx.send(Ok(None)).await;
        }
        .in_current_span(),
    );

    Ok(CompletionsResponse::Streaming(response_rx))
}

#[instrument(skip_all)]
async fn handle_input_detection(
    ctx: Arc<Context>,
    task: &CompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
) -> Result<Option<Completion>, Error> {
    let trace_id = task.trace_id;
    let model_id = task.request.model.clone();

    let input_id = 0;
    let input_text = task.request.prompt.clone();
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
        // Build completion chunk with input detections
        let chunk = Completion {
            id: Uuid::new_v4().simple().to_string(),
            model: model_id,
            created: common::current_timestamp().as_secs() as i64,
            detections: Some(OpenAiDetections {
                input: vec![InputDetectionResult {
                    message_index: input_id,
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
        Ok(Some(chunk))
    } else {
        // No input detections
        Ok(None)
    }
}

#[instrument(skip_all)]
async fn handle_output_detection(
    ctx: Arc<Context>,
    task: &CompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
    completion_stream: CompletionStream,
    response_tx: mpsc::Sender<Result<Option<Completion>, Error>>,
) {
    let trace_id = task.trace_id;
    let request = task.request.clone();
    // Split output detectors into 2 groups:
    // 1) Output Detectors: Applied to chunks. Detections are returned in batches.
    // 2) Whole Doc Output Detectors: Applied to concatenated chunks (whole doc) after the completion stream has been consumed.
    // Currently, this is any detector that uses "whole_doc_chunker".
    let (whole_doc_detectors, detectors): (HashMap<_, _>, HashMap<_, _>) =
        detectors.into_iter().partition(|(detector_id, _)| {
            ctx.config.get_chunker_id(detector_id).unwrap() == "whole_doc_chunker"
        });
    let completion_state = Arc::new(CompletionState::new());

    if !detectors.is_empty() {
        // Set up streaming detection pipeline
        // n represents how many choices to generate for each input message
        // Choices are processed independently so each choice has its own input channels and detection streams.
        let n = request.extra.get("n").and_then(|v| v.as_i64()).unwrap_or(1) as usize;
        // Create input channels
        let mut input_txs = HashMap::with_capacity(n);
        let mut input_rxs = HashMap::with_capacity(n);
        (0..n).for_each(|choice_index| {
            let (input_tx, input_rx) = mpsc::channel::<Result<(usize, String), Error>>(32);
            input_txs.insert(choice_index as u32, input_tx);
            input_rxs.insert(choice_index as u32, input_rx);
        });
        // Create detection streams
        let mut detection_streams = Vec::with_capacity(n * detectors.len());
        for (choice_index, input_rx) in input_rxs {
            match common::text_contents_detection_streams(
                ctx.clone(),
                task.headers.clone(),
                detectors.clone(),
                choice_index,
                input_rx,
            )
            .await
            {
                Ok(streams) => {
                    detection_streams.extend(streams);
                }
                Err(error) => {
                    error!(%trace_id, %error, "task failed: error creating detection streams");
                    // Send error to response channel and terminate
                    let _ = response_tx.send(Err(error)).await;
                }
            }
        }

        // Spawn task to consume completions stream and send choice text to detection pipeline
        tokio::spawn(process_completion_stream(
            trace_id,
            completion_stream,
            Some(completion_state.clone()),
            Some(input_txs),
            None,
        ));
        // Process detection streams and await completion
        let detection_batch_stream =
            DetectionBatchStream::new(CompletionBatcher::new(detectors.len()), detection_streams);
        process_detection_batch_stream(
            trace_id,
            completion_state.clone(),
            detection_batch_stream,
            response_tx.clone(),
        )
        .await;
    } else {
        // We only have whole doc detectors, so the streaming detection pipeline is disabled
        // Consume completions stream and await completion
        process_completion_stream(
            trace_id,
            completion_stream,
            Some(completion_state.clone()),
            None,
            Some(response_tx.clone()),
        )
        .await;
    }
    // NOTE: at this point, the completions stream has been fully consumed and completion state is final

    // If whole doc output detections or usage is requested, a final message is sent with these items
    if !whole_doc_detectors.is_empty() || completion_state.usage().is_some() {
        let mut completion = Completion {
            id: completion_state.id().unwrap().to_string(),
            created: completion_state.created().unwrap(),
            model: completion_state.model().unwrap().to_string(),
            usage: completion_state.usage().cloned(),
            ..Default::default()
        };
        if !whole_doc_detectors.is_empty() {
            // Handle whole doc output detection
            match handle_whole_doc_output_detection(
                ctx.clone(),
                task,
                whole_doc_detectors,
                completion_state,
            )
            .await
            {
                Ok((detections, warnings)) => {
                    completion.detections = Some(detections);
                    completion.warnings = warnings;
                }
                Err(error) => {
                    error!(%error, "task failed: error processing whole doc output detections");
                    // Send error to response channel
                    let _ = response_tx.send(Err(error)).await;
                    return;
                }
            }
        }
        // Send completion with whole doc output detections and/or usage to response channel
        let _ = response_tx.send(Ok(Some(completion))).await;
    }
}

/// Processes completion stream.
#[allow(clippy::type_complexity)]
async fn process_completion_stream(
    trace_id: TraceId,
    mut completion_stream: CompletionStream,
    completion_state: Option<Arc<CompletionState<Completion>>>,
    input_txs: Option<HashMap<u32, mpsc::Sender<Result<(usize, String), Error>>>>,
    response_tx: Option<mpsc::Sender<Result<Option<Completion>, Error>>>,
) {
    while let Some((message_index, result)) = completion_stream.next().await {
        match result {
            Ok(Some(completion)) => {
                // Send completion chunk to response channel
                // NOTE: this forwards completion chunks without detections and is only
                // done here for 2 cases: a) no output detectors b) only whole doc output detectors
                if let Some(response_tx) = &response_tx {
                    if response_tx
                        .send(Ok(Some(completion.clone())))
                        .await
                        .is_err()
                    {
                        info!(%trace_id, "task completed: client disconnected");
                        return;
                    }
                }
                if let Some(usage) = &completion.usage
                    && completion.choices.is_empty()
                {
                    // Update state: set usage
                    // NOTE: this message has no choices and is not sent to detection input channel
                    if let Some(state) = &completion_state {
                        state.set_usage(usage.clone());
                    }
                } else {
                    if message_index == 0 {
                        // Update state: set metadata
                        // NOTE: these values are the same for all completion chunks
                        if let Some(state) = &completion_state {
                            state.set_metadata(
                                completion.id.clone(),
                                completion.created,
                                completion.model.clone(),
                            );
                        }
                    }
                    // NOTE: completion chunks should contain only 1 choice
                    if let Some(choice) = completion.choices.first() {
                        // Extract choice text
                        let choice_text = choice.text.clone();
                        // Update state: insert completion
                        if let Some(state) = &completion_state {
                            state.insert_completion(
                                choice.index,
                                message_index,
                                completion.clone(),
                            );
                        }
                        // Send choice text to detection input channel
                        if let Some(input_tx) =
                            input_txs.as_ref().and_then(|txs| txs.get(&choice.index))
                        {
                            if !choice_text.is_empty() {
                                let _ = input_tx.send(Ok((message_index, choice_text))).await;
                            }
                        }
                    } else {
                        debug!(%trace_id, %message_index, ?completion, "completion chunk contains no choice");
                        warn!(%trace_id, %message_index, "completion chunk contains no choice");
                    }
                }
            }
            Ok(None) => (), // Complete, stream has closed
            Err(error) => {
                error!(%trace_id, %error, "task failed: error received from completion stream");
                // Send error to response channel
                if let Some(response_tx) = &response_tx {
                    let _ = response_tx.send(Err(error.clone())).await;
                }
                // Send error to detection input channels
                if let Some(input_txs) = &input_txs {
                    for input_tx in input_txs.values() {
                        let _ = input_tx.send(Err(error.clone())).await;
                    }
                }
            }
        }
    }
}

#[instrument(skip_all)]
async fn handle_whole_doc_output_detection(
    ctx: Arc<Context>,
    task: &CompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
    completion_state: Arc<CompletionState<Completion>>,
) -> Result<(OpenAiDetections, Vec<OrchestratorWarning>), Error> {
    // Create vec of choice_index->inputs, where inputs contains the concatenated text for the choice
    let choice_inputs = completion_state
        .completions
        .iter()
        .map(|entry| {
            let choice_index = *entry.key();
            let text = entry
                .values()
                .map(|chunk| {
                    chunk
                        .choices
                        .first()
                        .map(|choice| choice.text.clone())
                        .unwrap_or_default()
                })
                .collect::<String>();
            let inputs = vec![(0usize, text)];
            (choice_index, inputs)
        })
        .collect::<Vec<_>>();
    // Process detections concurrently for choices
    let choice_detections = stream::iter(choice_inputs)
        .map(|(choice_index, inputs)| {
            text_contents_detections(
                ctx.clone(),
                task.headers.clone(),
                detectors.clone(),
                choice_index,
                inputs,
            )
        })
        .buffer_unordered(ctx.config.detector_concurrent_requests)
        .try_collect::<Vec<_>>()
        .await?;
    // Build output detections
    let output = choice_detections
        .into_iter()
        .map(|(choice_index, detections)| OutputDetectionResult {
            choice_index,
            results: detections.into(),
        })
        .collect::<Vec<_>>();
    // Build warnings
    let warnings = if output.iter().any(|d| !d.results.is_empty()) {
        vec![OrchestratorWarning::new(
            DetectionWarningReason::UnsuitableOutput,
            UNSUITABLE_OUTPUT_MESSAGE,
        )]
    } else {
        Vec::new()
    };
    let detections = OpenAiDetections {
        output,
        ..Default::default()
    };
    Ok((detections, warnings))
}

/// Builds a response with output detections.
fn output_detection_response(
    completion_state: &Arc<CompletionState<Completion>>,
    choice_index: u32,
    chunk: Chunk,
    detections: Detections,
) -> Result<Completion, Error> {
    // Get completions for this choice index
    let completions = completion_state.completions.get(&choice_index).unwrap();
    // Get range of completions for this chunk
    let completions = completions
        .range(chunk.input_start_index..=chunk.input_end_index)
        .map(|(_index, completion)| completion.clone())
        .collect::<Vec<_>>();
    let logprobs = merge_logprobs(&completions);
    // Build response using the last completion received for this chunk
    if let Some(completion) = completions.last() {
        let mut completion = completion.clone();
        // Set content
        completion.choices[0].text = chunk.text;
        // Set logprobs
        completion.choices[0].logprobs = logprobs;
        // Set warnings
        if !detections.is_empty() {
            completion.warnings = vec![OrchestratorWarning::new(
                DetectionWarningReason::UnsuitableOutput,
                UNSUITABLE_OUTPUT_MESSAGE,
            )];
        }
        // Set detections
        completion.detections = Some(OpenAiDetections {
            output: vec![OutputDetectionResult {
                choice_index,
                results: detections.into(),
            }],
            ..Default::default()
        });
        Ok(completion)
    } else {
        error!(
            %choice_index,
            %chunk.input_start_index,
            %chunk.input_end_index,
            "no completions found for chunk"
        );
        Err(Error::Other("no completions found for chunk".into()))
    }
}

/// Combines logprobs from completion chunks to a single [`CompletionLogprobs`].
fn merge_logprobs(completions: &[Completion]) -> Option<CompletionLogprobs> {
    let mut merged_logprobs = CompletionLogprobs::default();
    for completion in completions {
        if let Some(choice) = completion.choices.first() {
            if let Some(logprobs) = &choice.logprobs {
                merged_logprobs.tokens.extend_from_slice(&logprobs.tokens);
                merged_logprobs
                    .token_logprobs
                    .extend_from_slice(&logprobs.token_logprobs);
                merged_logprobs
                    .top_logprobs
                    .extend_from_slice(&logprobs.top_logprobs);
                merged_logprobs
                    .text_offset
                    .extend_from_slice(&logprobs.text_offset);
            }
        }
    }
    (!merged_logprobs.tokens.is_empty()
        || !merged_logprobs.token_logprobs.is_empty()
        || !merged_logprobs.top_logprobs.is_empty()
        || !merged_logprobs.text_offset.is_empty())
    .then_some(merged_logprobs)
}

/// Consumes a detection batch stream, builds responses, and sends them to a response channel.
async fn process_detection_batch_stream(
    trace_id: TraceId,
    completion_state: Arc<CompletionState<Completion>>,
    mut detection_batch_stream: DetectionBatchStream,
    response_tx: mpsc::Sender<Result<Option<Completion>, Error>>,
) {
    while let Some(result) = detection_batch_stream.next().await {
        match result {
            Ok((choice_index, chunk, detections)) => {
                let input_end_index = chunk.input_end_index;
                match output_detection_response(&completion_state, choice_index, chunk, detections)
                {
                    Ok(completion) => {
                        // Send completion to response channel
                        debug!(%trace_id, %choice_index, ?completion, "sending completion chunk to response channel");
                        if response_tx.send(Ok(Some(completion))).await.is_err() {
                            info!(%trace_id, "task completed: client disconnected");
                            return;
                        }
                        // If this is the final completion chunk with content, send completion chunk with finish reason
                        let completions = completion_state.completions.get(&choice_index).unwrap();
                        if completions.keys().rev().nth(1) == Some(&input_end_index) {
                            if let Some((_, completion)) = completions.last_key_value() {
                                if completion
                                    .choices
                                    .first()
                                    .is_some_and(|choice| choice.finish_reason.is_some())
                                {
                                    debug!(%trace_id, %choice_index, ?completion, "sending completion chunk with finish reason to response channel");
                                    let _ = response_tx.send(Ok(Some(completion.clone()))).await;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        error!(%trace_id, %error, "task failed: error building output detection response");
                        // Send error to response channel and terminate
                        let _ = response_tx.send(Err(error)).await;
                        return;
                    }
                }
            }
            Err(error) => {
                error!(%trace_id, %error, "task failed: error received from detection batch stream");
                // Send error to response channel and terminate
                let _ = response_tx.send(Err(error)).await;
                return;
            }
        }
    }
    info!(%trace_id, "task completed: detection batch stream closed");
}
