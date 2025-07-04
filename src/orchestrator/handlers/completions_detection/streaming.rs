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
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use dashmap::DashMap;
use futures::{StreamExt, TryStreamExt, stream};
use opentelemetry::trace::TraceId;
use tokio::sync::mpsc;
use tracing::{Instrument, debug, error, info, instrument, warn};
use uuid::Uuid;

use super::CompletionsDetectionTask;
use crate::{
    clients::openai::*,
    models::{
        DetectionWarningReason, DetectorParams, UNSUITABLE_INPUT_MESSAGE, UNSUITABLE_OUTPUT_MESSAGE,
    },
    orchestrator::{
        Context, Error,
        common::{self, text_contents_detections},
        types::{
            ChatCompletionBatcher, ChatCompletionStream, ChoiceIndex, Chunk, DetectionBatchStream,
            Detections,
        },
    },
};

pub async fn handle_streaming(
    _ctx: Arc<Context>,
    task: CompletionsDetectionTask,
) -> Result<CompletionsResponse, Error> {
    let trace_id = task.trace_id;
    let detectors = task.request.detectors.clone();
    info!(%trace_id, config = ?detectors, "task started");

    // Create response channel
    let (response_tx, response_rx) = mpsc::channel::<Result<Option<Completion>, Error>>(128);

    tokio::spawn(
        async move {
            // TODO
            let _ = response_tx
                .send(Err(Error::Validation(
                    "streaming is not yet supported".into(),
                )))
                .await;
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
) -> Result<Option<ChatCompletionChunk>, Error> {
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
        // Build chat completion chunk with input detections
        let chunk = ChatCompletionChunk {
            id: Uuid::new_v4().simple().to_string(),
            model: model_id,
            created: common::current_timestamp().as_secs() as i64,
            detections: Some(ChatDetections {
                input: vec![InputDetectionResult {
                    message_index: input_id, // TODO:double-check this
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
    chat_completion_stream: ChatCompletionStream,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    let trace_id = task.trace_id;
    let request = task.request.clone();
    // Split output detectors into 2 groups:
    // 1) Output Detectors: Applied to chunks. Detections are returned in batches.
    // 2) Whole Doc Output Detectors: Applied to concatenated chunks (whole doc) after the chat completion stream has been consumed.
    // Currently, this is any detector that uses "whole_doc_chunker".
    let (whole_doc_detectors, detectors): (HashMap<_, _>, HashMap<_, _>) =
        detectors.into_iter().partition(|(detector_id, _)| {
            ctx.config.get_chunker_id(detector_id).unwrap() == "whole_doc_chunker"
        });
    let chat_completion_state = Arc::new(_ChatCompletionState::_new());

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

        // Spawn task to consume chat completions stream and send choice text to detection pipeline
        tokio::spawn(_process_chat_completion_stream(
            trace_id,
            chat_completion_stream,
            Some(chat_completion_state.clone()),
            Some(input_txs),
            None,
        ));
        // Process detection streams and await completion
        let detection_batch_stream = DetectionBatchStream::new(
            ChatCompletionBatcher::new(detectors.len()),
            detection_streams,
        );
        _process_detection_batch_stream(
            trace_id,
            chat_completion_state.clone(),
            detection_batch_stream,
            response_tx.clone(),
        )
        .await;
    } else {
        // We only have whole doc detectors, so the streaming detection pipeline is disabled
        // Consume chat completions stream and await completion
        _process_chat_completion_stream(
            trace_id,
            chat_completion_stream,
            Some(chat_completion_state.clone()),
            None,
            Some(response_tx.clone()),
        )
        .await;
    }
    // NOTE: at this point, the chat completions stream has been fully consumed and chat completion state is final

    // If whole doc output detections or usage is requested, a final message is sent with these items
    if !whole_doc_detectors.is_empty() || chat_completion_state._usage().is_some() {
        let mut chat_completion = ChatCompletionChunk {
            id: chat_completion_state._id(),
            created: chat_completion_state._created(),
            model: chat_completion_state._model(),
            usage: chat_completion_state._usage(),
            ..Default::default()
        };
        if !whole_doc_detectors.is_empty() {
            // Handle whole doc output detection
            match handle_whole_doc_output_detection(
                ctx.clone(),
                task,
                whole_doc_detectors,
                chat_completion_state,
            )
            .await
            {
                Ok((detections, warnings)) => {
                    chat_completion.detections = Some(detections);
                    chat_completion.warnings = warnings;
                }
                Err(error) => {
                    error!(%error, "task failed: error processing whole doc output detections");
                    // Send error to response channel
                    let _ = response_tx.send(Err(error)).await;
                    return;
                }
            }
        }
        // Send chat completion with whole doc output detections and/or usage to response channel
        let _ = response_tx.send(Ok(Some(chat_completion))).await;
    }
}

/// Processes chat completion stream.
#[allow(clippy::type_complexity)]
async fn _process_chat_completion_stream(
    trace_id: TraceId,
    mut chat_completion_stream: ChatCompletionStream,
    chat_completion_state: Option<Arc<_ChatCompletionState>>,
    input_txs: Option<HashMap<u32, mpsc::Sender<Result<(usize, String), Error>>>>,
    response_tx: Option<mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>>,
) {
    while let Some((message_index, result)) = chat_completion_stream.next().await {
        match result {
            Ok(Some(chat_completion)) => {
                // Send chat completion chunk to response channel
                // NOTE: this forwards chat completion chunks without detections and is only
                // done here for 2 cases: a) no output detectors b) only whole doc output detectors
                if let Some(response_tx) = &response_tx {
                    if response_tx
                        .send(Ok(Some(chat_completion.clone())))
                        .await
                        .is_err()
                    {
                        info!(%trace_id, "task completed: client disconnected");
                        return;
                    }
                }
                if chat_completion.usage.is_some() {
                    // Set usage state from the usage message
                    // NOTE: this message has no choices and is not sent to detection input channel
                    if let Some(state) = &chat_completion_state {
                        state.metadata.lock().unwrap().usage = chat_completion.usage.clone();
                    }
                } else {
                    if message_index == 0 {
                        // Set metadata state from the first message
                        // NOTE: these values are the same for all chat completion chunks
                        if let Some(state) = &chat_completion_state {
                            let mut metadata = state.metadata.lock().unwrap();
                            metadata.id = chat_completion.id.clone();
                            metadata.created = chat_completion.created;
                            metadata.model = chat_completion.model.clone();
                        }
                    }
                    // NOTE: chat completion chunks should contain only 1 choice
                    if let Some(choice) = chat_completion.choices.first() {
                        // Extract choice text
                        let choice_text = choice.delta.content.clone().unwrap_or_default();
                        // Update state for this choice index
                        if let Some(state) = &chat_completion_state {
                            match state.chat_completions.entry(choice.index) {
                                dashmap::Entry::Occupied(mut entry) => {
                                    entry
                                        .get_mut()
                                        .insert(message_index, chat_completion.clone());
                                }
                                dashmap::Entry::Vacant(entry) => {
                                    entry.insert(BTreeMap::from([(
                                        message_index,
                                        chat_completion.clone(),
                                    )]));
                                }
                            }
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
                        debug!(%trace_id, %message_index, ?chat_completion, "chat completion chunk contains no choice");
                        warn!(%trace_id, %message_index, "chat completion chunk contains no choice");
                    }
                }
            }
            Ok(None) => (), // Complete, stream has closed
            Err(error) => {
                error!(%trace_id, %error, "task failed: error received from chat completion stream");
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
    chat_completion_state: Arc<_ChatCompletionState>,
) -> Result<(ChatDetections, Vec<OrchestratorWarning>), Error> {
    // Create vec of choice_index->inputs, where inputs contains the concatenated text for the choice
    let choice_inputs = chat_completion_state
        .chat_completions
        .iter()
        .map(|entry| {
            let choice_index = *entry.key();
            let text = entry
                .values()
                .map(|chunk| {
                    chunk
                        .choices
                        .first()
                        .and_then(|choice| choice.delta.content.clone())
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
    let detections = ChatDetections {
        output,
        ..Default::default()
    };
    Ok((detections, warnings))
}

/// Builds a response with output detections.
fn _output_detection_response(
    chat_completion_state: &Arc<_ChatCompletionState>,
    choice_index: u32,
    chunk: Chunk,
    detections: Detections,
) -> Result<ChatCompletionChunk, Error> {
    // Get chat completions for this choice index
    let chat_completions = chat_completion_state
        .chat_completions
        .get(&choice_index)
        .unwrap();
    // Get range of chat completions for this chunk
    let chat_completions = chat_completions
        .range(chunk.input_start_index..=chunk.input_end_index)
        .map(|(_index, chat_completion)| chat_completion.clone())
        .collect::<Vec<_>>();
    let content = Some(chunk.text);
    let logprobs = _merge_logprobs(&chat_completions);
    // Build response using the last chat completion received for this chunk
    if let Some(chat_completion) = chat_completions.last() {
        let mut chat_completion = chat_completion.clone();
        // Set role
        chat_completion.choices[0].delta.role = Some(Role::Assistant);
        // Set content
        chat_completion.choices[0].delta.content = content;
        // TODO: if applicable, set tool_calls and refusal
        // Set logprobs
        chat_completion.choices[0].logprobs = logprobs;
        // Set warnings
        if !detections.is_empty() {
            chat_completion.warnings = vec![OrchestratorWarning::new(
                DetectionWarningReason::UnsuitableOutput,
                UNSUITABLE_OUTPUT_MESSAGE,
            )];
        }
        // Set detections
        chat_completion.detections = Some(ChatDetections {
            output: vec![OutputDetectionResult {
                choice_index,
                results: detections.into(),
            }],
            ..Default::default()
        });
        Ok(chat_completion)
    } else {
        error!(
            %choice_index,
            %chunk.input_start_index,
            %chunk.input_end_index,
            "no chat completions found for chunk"
        );
        Err(Error::Other("no chat completions found for chunk".into()))
    }
}

/// Combines logprobs from chat completion chunks to a single [`ChatCompletionLogprobs`].
fn _merge_logprobs(chat_completions: &[ChatCompletionChunk]) -> Option<ChatCompletionLogprobs> {
    let mut content: Vec<ChatCompletionLogprob> = Vec::new();
    let mut refusal: Vec<ChatCompletionLogprob> = Vec::new();
    for chat_completion in chat_completions {
        if let Some(choice) = chat_completion.choices.first() {
            if let Some(logprobs) = &choice.logprobs {
                content.extend_from_slice(&logprobs.content);
                refusal.extend_from_slice(&logprobs.refusal);
            }
        }
    }
    (!content.is_empty() || !refusal.is_empty())
        .then_some(ChatCompletionLogprobs { content, refusal })
}

/// Consumes a detection batch stream, builds responses, and sends them to a response channel.
async fn _process_detection_batch_stream(
    trace_id: TraceId,
    chat_completion_state: Arc<_ChatCompletionState>,
    mut detection_batch_stream: DetectionBatchStream,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    while let Some(result) = detection_batch_stream.next().await {
        match result {
            Ok((choice_index, chunk, detections)) => {
                let input_end_index = chunk.input_end_index;
                match _output_detection_response(
                    &chat_completion_state,
                    choice_index,
                    chunk,
                    detections,
                ) {
                    Ok(chat_completion) => {
                        // Send chat completion to response channel
                        debug!(%trace_id, %choice_index, ?chat_completion, "sending chat completion chunk to response channel");
                        if response_tx.send(Ok(Some(chat_completion))).await.is_err() {
                            info!(%trace_id, "task completed: client disconnected");
                            return;
                        }
                        // If this is the final chat completion chunk with content, send chat completion chunk with finish reason
                        let chat_completions = chat_completion_state
                            .chat_completions
                            .get(&choice_index)
                            .unwrap();
                        if chat_completions.keys().rev().nth(1) == Some(&input_end_index) {
                            if let Some((_, chat_completion)) = chat_completions.last_key_value() {
                                if chat_completion
                                    .choices
                                    .first()
                                    .is_some_and(|choice| choice.finish_reason.is_some())
                                {
                                    let mut chat_completion = chat_completion.clone();
                                    // Set role
                                    chat_completion.choices[0].delta.role = Some(Role::Assistant);
                                    debug!(%trace_id, %choice_index, ?chat_completion, "sending chat completion chunk with finish reason to response channel");
                                    let _ = response_tx.send(Ok(Some(chat_completion))).await;
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

#[derive(Debug, Default)]
struct _ChatCompletionMetadata {
    /// A unique identifier for the chat completion. Each chunk has the same ID.
    pub id: String,
    /// The Unix timestamp (in seconds) of when the chat completion was created. Each chunk has the same timestamp.
    pub created: i64,
    /// The model to generate the completion.
    pub model: String,
    /// Completion usage statistics.
    pub usage: Option<Usage>,
}

#[derive(Debug, Default)]
struct _ChatCompletionState {
    /// Chat completion metadata.
    pub metadata: Mutex<_ChatCompletionMetadata>,
    /// A map of chat completion chunks received for each choice.
    pub chat_completions: DashMap<ChoiceIndex, BTreeMap<usize, ChatCompletionChunk>>,
}

impl _ChatCompletionState {
    pub fn _new() -> Self {
        Self::default()
    }

    pub fn _id(&self) -> String {
        self.metadata.lock().unwrap().id.clone()
    }

    pub fn _created(&self) -> i64 {
        self.metadata.lock().unwrap().created
    }

    pub fn _model(&self) -> String {
        self.metadata.lock().unwrap().model.clone()
    }

    pub fn _usage(&self) -> Option<Usage> {
        self.metadata.lock().unwrap().usage.clone()
    }
}
