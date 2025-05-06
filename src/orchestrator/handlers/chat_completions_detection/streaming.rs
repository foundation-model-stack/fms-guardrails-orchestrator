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
use std::{collections::HashMap, sync::{Arc, RwLock}};

use futures::StreamExt;
use opentelemetry::trace::TraceId;
use tokio::sync::mpsc;
use tracing::{Instrument, error, info, instrument};
use uuid::Uuid;

use super::ChatCompletionsDetectionTask;
use crate::{
    clients::openai::*,
    config::DetectorType,
    models::{DetectionWarningReason, DetectorParams, UNSUITABLE_INPUT_MESSAGE},
    orchestrator::{
        common::{self, validate_detectors}, types::{ChatCompletionBatcher, ChatCompletionStream, ChatMessageIterator, DetectionBatchStream, DetectionStream}, Context, Error
    },
};

pub async fn handle_streaming(
    ctx: Arc<Context>,
    task: ChatCompletionsDetectionTask,
) -> Result<ChatCompletionsResponse, Error> {
    let trace_id = task.trace_id;
    let detectors = task.request.detectors.clone();
    info!(%trace_id, config = ?detectors, "task started");

    // Create response channel
    let (response_tx, response_rx) =
        mpsc::channel::<Result<Option<ChatCompletionChunk>, Error>>(128);

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
                        return;
                    }
                }
            }

            // Create chat completions stream
            let client = ctx
                .clients
                .get_as::<OpenAiClient>("chat_generation")
                .unwrap();
            let chat_completion_stream =
                match common::chat_completion_stream(client, task.headers.clone(), task.request.clone()).await {
                    Ok(stream) => stream,
                    Err(error) => {
                        error!(%trace_id, %error, "task failed: error creating chat completions stream");
                        // Send error to response channel and terminate
                        let _ = response_tx.send(Err(error)).await;
                        return;
                    }
                };
            
            if output_detectors.is_empty() {
                // No output detectors, forward chat completion chunks to response channel
                forward_chat_completion_stream(trace_id, chat_completion_stream, response_tx.clone()).await;
            } else {
                // Partition output detectors
                // Detectors using whole_doc_chunker are processed at the end after all chat completion chunks
                // have been collected. Results are returned with the second-last message.
                let (whole_doc_output_detectors, output_detectors): (HashMap<_, _>, HashMap<_, _>) = output_detectors
                    .into_iter()
                    .partition(|(detector_id, _params)| {
                        let chunker_id = ctx
                            .config
                            .get_chunker_id(detector_id)
                            .unwrap();
                        chunker_id == "whole_doc_chunker"
                    });

                // Create shared chat completions
                let chat_completions: Arc<RwLock<Vec<ChatCompletionChunk>>> = Arc::new(RwLock::new(Vec::new()));

                // Handle output detection
                handle_output_detection(
                    ctx.clone(),
                    &task,
                    output_detectors,
                    chat_completions.clone(),
                    chat_completion_stream,
                    response_tx.clone(),
                )
                .await;

                // Handle whole doc output detection
                handle_whole_doc_output_detection(
                    ctx.clone(),
                    &task,
                    whole_doc_output_detectors,
                    chat_completions.clone(),
                    response_tx.clone(),
                )
                .await;
            }

            // Send None to signal completion
            let _ = response_tx.send(Ok(None)).await;
        }
        .in_current_span(),
    );

    Ok(ChatCompletionsResponse::Streaming(response_rx))
}

#[instrument(skip_all)]
async fn handle_input_detection(
    ctx: Arc<Context>,
    task: &ChatCompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
) -> Result<Option<ChatCompletionChunk>, Error> {
    let trace_id = task.trace_id;
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
        // Build chat completion chunk with input detections
        let chunk = ChatCompletionChunk {
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
        Ok(Some(chunk))
    } else {
        // No input detections
        Ok(None)
    }
}


#[instrument(skip_all)]
async fn handle_output_detection(
    ctx: Arc<Context>,
    task: &ChatCompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
    chat_completions: Arc<RwLock<Vec<ChatCompletionChunk>>>,
    chat_completion_stream: ChatCompletionStream,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    let trace_id = &task.trace_id;
    let request = task.request.clone();
    let n = request.extra.get("n").and_then(|v| v.as_i64()).unwrap_or(1) as usize;

    // Create input channels
    let mut input_senders = HashMap::with_capacity(n);
    let mut input_receivers = HashMap::with_capacity(n);
    (1..=n).for_each(|choice_index| {
        let (input_tx, input_rx) = mpsc::channel::<Result<(usize, String), Error>>(32);
        input_senders.insert(choice_index as u32, input_tx);
        input_receivers.insert(choice_index as u32, input_rx);
    });

    // Spawn task to process chat completions
    // Handles sending chunk choices to their respective input channels
    // and updating shared chat completions.
    tokio::spawn(process_chat_completion_stream(
        chat_completions.clone(),
        chat_completion_stream,
        input_senders,
    ));

    // Create detection streams
    let mut detection_streams = Vec::with_capacity(n * detectors.len());
    for (input_id, input_rx) in input_receivers {
        match common::text_contents_detection_streams(
            ctx.clone(),
            task.headers.clone(),
            detectors.clone(),
            input_id,
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

    if detection_streams.len() == 1 {
        // Process single detection stream, batching not applicable
        let detection_stream = detection_streams.swap_remove(1);
        process_detection_stream(trace_id, chat_completions, detection_stream, response_tx).await;
    } else {
        // Create detection batch stream
        let detection_batch_stream =
            DetectionBatchStream::new(ChatCompletionBatcher::new(detectors.len()), detection_streams);
        process_detection_batch_stream(
            trace_id,
            chat_completions,
            detection_batch_stream,
            response_tx,
        )
        .await;
    }
}

#[instrument(skip_all)]
async fn handle_whole_doc_output_detection(
    _ctx: Arc<Context>,
    _task: &ChatCompletionsDetectionTask,
    _detectors: HashMap<String, DetectorParams>,
    _chat_completions: Arc<RwLock<Vec<ChatCompletionChunk>>>,
    _response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    todo!()
}

async fn forward_chat_completion_stream(
    trace_id: TraceId,
    mut chat_completion_stream: ChatCompletionStream,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    while let Some((_index, result)) = chat_completion_stream.next().await {
        match result {
            Ok(Some(chat_completion)) => {
                // Send message to response channel
                if response_tx.send(Ok(Some(chat_completion))).await.is_err() {
                    info!(%trace_id, "task completed: client disconnected");
                    return;
                }
            }
            Ok(None) => {
                // Send message to response channel
                if response_tx.send(Ok(None)).await.is_err() {
                    info!(%trace_id, "task completed: client disconnected");
                    return;
                }
            }
            Err(error) => {
                error!(%trace_id, %error, "task failed: error received from chat completion stream");
                // Send error to response channel and terminate
                let _ = response_tx.send(Err(error)).await;
                return;
            }
        }
    }
    info!(%trace_id, "task completed: chat completion stream closed");
}

#[allow(clippy::type_complexity)]
async fn process_chat_completion_stream(
    chat_completions: Arc<RwLock<Vec<ChatCompletionChunk>>>,
    mut chat_completion_stream: ChatCompletionStream,
    input_senders: HashMap<u32, mpsc::Sender<Result<(usize, String), Error>>>,
) {
    while let Some((index, result)) = chat_completion_stream.next().await {
        match result {
            Ok(Some(completion)) => {
                for choice in &completion.choices {
                    // Send generated text to input channel
                    let input = (index, choice.clone().delta.content.unwrap_or_default());
                    let input_tx = input_senders.get(&choice.index).unwrap();
                    let _ = input_tx.send(Ok(input)).await;
                }
                // Update shared chat completions
                chat_completions.write().unwrap().push(completion);
            }
            Ok(None) => (),
            Err(error) => {
                // Send error to all input channels
                for input_tx in input_senders.values() {
                    let _ = input_tx.send(Err(error.clone())).await;
                }
            }
        }
    }
}

/// Consumes a detection stream, builds responses, and sends them to a response channel.
async fn process_detection_stream(
    trace_id: &TraceId,
    _chat_completions: Arc<RwLock<Vec<ChatCompletionChunk>>>,
    mut detection_stream: DetectionStream,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    while let Some(result) = detection_stream.next().await {
        match result {
            Ok((_choice_index, _detector_id, _chunk, _detections)) => {
                todo!()
            }
            Err(error) => {
                error!(%trace_id, %error, "task failed: error received from detection stream");
                // Send error to response channel and terminate
                let _ = response_tx.send(Err(error)).await;
                return;
            }
        }
    }
    info!(%trace_id, "task completed: detection stream closed");
}

/// Consumes a detection batch stream, builds responses, and sends them to a response channel.
async fn process_detection_batch_stream(
    trace_id: &TraceId,
    _chat_completions: Arc<RwLock<Vec<ChatCompletionChunk>>>,
    mut detection_batch_stream: DetectionBatchStream<ChatCompletionBatcher>,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    while let Some(result) = detection_batch_stream.next().await {
        match result {
            Ok(_batch) => {
                // Create response for this batch with output detections
                todo!()
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