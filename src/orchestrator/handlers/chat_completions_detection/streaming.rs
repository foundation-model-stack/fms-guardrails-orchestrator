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
use std::{collections::HashMap, sync::Arc, time::Duration};

use futures::{StreamExt, future::try_join_all};
use opentelemetry::trace::TraceId;
use tokio::sync::mpsc;
use tracing::{Instrument, debug, error, info, instrument, warn};
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
        common::{self, group_detectors_by_type, validate_detectors},
        types::{
            ChatCompletionStream, ChatMessageIterator, Chunk, CompletionBatcher, CompletionState,
            Detection, DetectionBatchStream,
        },
    },
};

/// Timeout duration when waiting for completion state entries to become available.
/// This handles the race condition where detectors respond faster than the LLM stream inserts completions.
///
/// The waiting mechanism uses cooperative yielding via `tokio::task::yield_now()`, not busy waiting.
/// Each yield returns immediately in the normal case (microseconds), so this timeout only matters
/// if the generation server is exceptionally slow or experiencing issues.
const COMPLETION_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

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

            if let Err(error) = validate_detectors(
                input_detectors.iter(),
                &ctx.config.detectors,
                &[DetectorType::TextContents],
                true,
            ) {
                let _ = response_tx.send(Err(error)).await;
                // Send None to signal completion
                let _ = response_tx.send(Ok(None)).await;
                return;
            }
            if let Err(error) = validate_detectors(
                output_detectors.iter(),
                &ctx.config.detectors,
                &[DetectorType::TextContents, DetectorType::TextChat],
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

            // Create chat completions stream
            let client = ctx
                .clients
                .get::<OpenAiClient>("openai")
                .unwrap();
            let chat_completion_stream = match common::chat_completion_stream(client, task.headers.clone(), task.request.clone()).await {
                Ok(stream) => stream,
                Err(error) => {
                    error!(%trace_id, %error, "task failed: error creating chat completions stream");
                    // Send error to response channel and terminate
                    let _ = response_tx.send(Err(error)).await;
                    // Send None to signal completion
                    let _ = response_tx.send(Ok(None)).await;
                    return;
                }
            };

            if output_detectors.is_empty() {
                // No output detectors, forward chat completion chunks to response channel
                process_chat_completion_stream(trace_id, chat_completion_stream, None, None, Some(response_tx.clone())).await;
                info!(%trace_id, "task completed: chat completion stream closed");
            } else {
                // Handle output detection
                handle_output_detection(
                    ctx.clone(),
                    &task,
                    output_detectors,
                    chat_completion_stream,
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

        // Build chat completion chunk with input detections
        let chunk = ChatCompletionChunk {
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
            usage: Some(usage),
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
    chat_completion_stream: ChatCompletionStream,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    let trace_id = task.trace_id;
    let request = task.request.clone();

    // Split output detectors into two categories:
    //
    // chunk_detectors: detectors applied to generated text chunks, detections returned in batches.
    // criteria: text_contents detectors using a chunker (not using whole_doc_chunker)
    //
    // whole_doc_detectors: detectors applied to full generated text (+prompt), detections returned with last message.
    // criteria: text_contents detectors not using a chunker (whole_doc_chunker) and other supported detector types
    let (chunk_detectors, whole_doc_detectors): (HashMap<_, _>, HashMap<_, _>) =
        detectors.into_iter().partition(|(detector_id, _)| {
            let config = ctx.config.detector(detector_id).unwrap();
            matches!(config.r#type, DetectorType::TextContents)
                && config.chunker_id != "whole_doc_chunker"
        });

    let completion_state = Arc::new(CompletionState::new());

    if !chunk_detectors.is_empty() {
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
        let mut detection_streams = Vec::with_capacity(n * chunk_detectors.len());
        for (choice_index, input_rx) in input_rxs {
            match common::text_contents_detection_streams(
                ctx.clone(),
                task.headers.clone(),
                chunk_detectors.clone(),
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
        tokio::spawn(process_chat_completion_stream(
            trace_id,
            chat_completion_stream,
            Some(completion_state.clone()),
            Some(input_txs),
            None,
        ));
        // Process detection streams and await completion
        let detection_batch_stream = DetectionBatchStream::new(
            CompletionBatcher::new(chunk_detectors.len()),
            detection_streams,
        );
        process_detection_batch_stream(
            trace_id,
            completion_state.clone(),
            detection_batch_stream,
            response_tx.clone(),
        )
        .await;
    } else {
        // We only have whole doc detectors, so the streaming detection pipeline is disabled
        // Consume chat completions stream and await completion
        process_chat_completion_stream(
            trace_id,
            chat_completion_stream,
            Some(completion_state.clone()),
            None,
            Some(response_tx.clone()),
        )
        .await;
    }
    // NOTE: at this point, the chat completions stream has been fully consumed and chat completion state is final

    // If whole doc output detections or usage is requested, a final message is sent with these items
    if !whole_doc_detectors.is_empty() || completion_state.usage().is_some() {
        let mut chat_completion = ChatCompletionChunk {
            id: completion_state.id().unwrap().to_string(),
            created: completion_state.created().unwrap(),
            model: completion_state.model().unwrap().to_string(),
            usage: completion_state.usage().cloned(),
            ..Default::default()
        };
        if !whole_doc_detectors.is_empty() {
            // Handle whole doc detection
            match handle_whole_doc_detection(
                ctx.clone(),
                task,
                whole_doc_detectors,
                completion_state,
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
                    // Send None to signal completion
                    let _ = response_tx.send(Ok(None)).await;
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
async fn process_chat_completion_stream(
    trace_id: TraceId,
    mut chat_completion_stream: ChatCompletionStream,
    completion_state: Option<Arc<CompletionState<ChatCompletionChunk>>>,
    input_txs: Option<HashMap<u32, mpsc::Sender<Result<(usize, String), Error>>>>,
    response_tx: Option<mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>>,
) {
    while let Some((message_index, result)) = chat_completion_stream.next().await {
        match result {
            Ok(Some(chat_completion)) => {
                // Send chat completion chunk to response channel
                // NOTE: this forwards chat completion chunks without detections and is only
                // done here for 2 cases: a) no output detectors b) only whole doc output detectors
                if let Some(response_tx) = &response_tx
                    && response_tx
                        .send(Ok(Some(chat_completion.clone())))
                        .await
                        .is_err()
                {
                    info!(%trace_id, "task completed: client disconnected");
                    return;
                }
                if let Some(usage) = &chat_completion.usage
                    && chat_completion.choices.is_empty()
                {
                    // Update state: set usage
                    // NOTE: this message has no choices and is not sent to detection input channel
                    if let Some(state) = &completion_state {
                        state.set_usage(usage.clone());
                    }
                } else {
                    if message_index == 0 {
                        // Update state: set metadata
                        // NOTE: these values are the same for all chat completion chunks
                        if let Some(state) = &completion_state {
                            state.set_metadata(
                                chat_completion.id.clone(),
                                chat_completion.created,
                                chat_completion.model.clone(),
                            );
                        }
                    }
                    // NOTE: chat completion chunks should contain only 1 choice
                    if let Some(choice) = chat_completion.choices.first() {
                        // Extract choice text
                        let choice_text = choice.delta.content.clone().unwrap_or_default();
                        // Update state: insert completion
                        if let Some(state) = &completion_state {
                            state.insert_completion(
                                choice.index,
                                message_index,
                                chat_completion.clone(),
                            );
                        }
                        // Send choice text to detection input channel
                        if let Some(input_tx) =
                            input_txs.as_ref().and_then(|txs| txs.get(&choice.index))
                            && !choice_text.is_empty()
                        {
                            let _ = input_tx.send(Ok((message_index, choice_text))).await;
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
async fn handle_whole_doc_detection(
    ctx: Arc<Context>,
    task: &ChatCompletionsDetectionTask,
    detectors: HashMap<String, DetectorParams>,
    completion_state: Arc<CompletionState<ChatCompletionChunk>>,
) -> Result<(CompletionDetections, Vec<CompletionDetectionWarning>), Error> {
    use DetectorType::*;
    let detector_groups = group_detectors_by_type(&ctx, detectors);
    let headers = &task.headers;
    let messages = task.request.messages.as_slice();
    let tools = task.request.tools.as_ref().cloned().unwrap_or_default();
    let mut warnings = Vec::new();

    // Create vec of choice_index->message
    // NOTE: we build a message from chat completion chunks as it is required for text chat detectors.
    // Text contents detectors only require the content text.
    let choices = completion_state
        .completions
        .iter()
        .map(|entry| {
            let choice_index = *entry.key();
            let chunks = entry.values().cloned().collect::<Vec<_>>();
            let tool_calls = merge_tool_calls(&chunks);
            let content = merge_content(&chunks);
            let message = Message {
                role: Role::Assistant,
                content,
                tool_calls,
                ..Default::default()
            };
            (choice_index, message)
        })
        .collect::<Vec<_>>();

    // Spawn detection tasks
    let mut tasks = Vec::with_capacity(choices.len() * detector_groups.len());
    for (choice_index, message) in &choices {
        if !message.has_content() {
            // Add no content warning
            warnings.push(CompletionDetectionWarning::new(
                DetectionWarningReason::EmptyOutput,
                &format!("Choice of index {choice_index} has no content"),
            ));
        }
        for (detector_type, detectors) in &detector_groups {
            let detection_task = match detector_type {
                TextContents => match message.text() {
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
                        [messages, std::slice::from_ref(message)].concat(),
                        tools.clone(),
                    )
                    .in_current_span(),
                ),
                _ => unimplemented!(),
            };
            tasks.push((*choice_index, *detector_type, detection_task));
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

    // If there are any text contents detections, add unsuitable output warning
    let unsuitable_output = detections.iter().any(|(_, detector_type, detections)| {
        matches!(detector_type, DetectorType::TextContents) && !detections.is_empty()
    });
    if unsuitable_output {
        warnings.push(CompletionDetectionWarning::new(
            DetectionWarningReason::UnsuitableOutput,
            UNSUITABLE_OUTPUT_MESSAGE,
        ));
    }

    // Build output detections
    let output = detections
        .into_iter()
        .map(|(choice_index, _, detections)| CompletionOutputDetections {
            choice_index,
            results: detections,
        })
        .collect::<Vec<_>>();
    let detections = CompletionDetections {
        output,
        ..Default::default()
    };
    Ok((detections, warnings))
}

/// Builds a response with output detections.
async fn output_detection_response(
    completion_state: &Arc<CompletionState<ChatCompletionChunk>>,
    choice_index: u32,
    chunk: Chunk,
    detections: Vec<Detection>,
) -> Result<ChatCompletionChunk, Error> {
    // Wait for entry to exist (yields to other tasks until ready)
    let chat_completions = {
        let wait_for_entry = async {
            loop {
                if let Some(entry) = completion_state.completions.get(&choice_index) {
                    return entry;
                }
                tokio::task::yield_now().await;
            }
        };

        match tokio::time::timeout(COMPLETION_WAIT_TIMEOUT, wait_for_entry).await {
            Ok(entry) => entry,
            Err(_) => {
                return Err(Error::Other(format!(
                    "completion entry for choice_index {} not ready after {:?} timeout",
                    choice_index, COMPLETION_WAIT_TIMEOUT
                )));
            }
        }
    };
    // Get range of chat completions for this chunk
    let chat_completions = chat_completions
        .range(chunk.input_start_index..=chunk.input_end_index)
        .map(|(_index, chat_completion)| chat_completion.clone())
        .collect::<Vec<_>>();
    let content = Some(chunk.text);
    let logprobs = merge_logprobs(&chat_completions);
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
            chat_completion.warnings = vec![CompletionDetectionWarning::new(
                DetectionWarningReason::UnsuitableOutput,
                UNSUITABLE_OUTPUT_MESSAGE,
            )];
        }
        // Set detections
        chat_completion.detections = Some(CompletionDetections {
            output: vec![CompletionOutputDetections {
                choice_index,
                results: detections,
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

/// Builds [`ChatCompletionLogprobs`] from chat completion chunks containing logprobs.
fn merge_logprobs(chunks: &[ChatCompletionChunk]) -> Option<ChatCompletionLogprobs> {
    let mut content: Vec<ChatCompletionLogprob> = Vec::new();
    let mut refusal: Vec<ChatCompletionLogprob> = Vec::new();
    for chunk in chunks {
        if let Some(choice) = chunk.choices.first()
            && let Some(logprobs) = &choice.logprobs
        {
            content.extend_from_slice(&logprobs.content);
            refusal.extend_from_slice(&logprobs.refusal);
        }
    }
    (!content.is_empty() || !refusal.is_empty())
        .then_some(ChatCompletionLogprobs { content, refusal })
}

/// Builds [`Vec<ToolCall>`] from chat completion chunks containing tool call chunks.
/// Builds tool calls from chat completion chunk deltas with tool call chunks.
fn merge_tool_calls(chunks: &[ChatCompletionChunk]) -> Option<Vec<ToolCall>> {
    let mut tool_calls: HashMap<usize, ToolCall> = HashMap::new();
    for chunk in chunks {
        if let Some(choice) = chunk.choices.first() {
            for tc in &choice.delta.tool_calls {
                let index = tc.index.unwrap();
                let entry = tool_calls.entry(index).or_insert_with(|| ToolCall {
                    index: Some(index),
                    ..Default::default()
                });
                if !tc.id.is_empty() {
                    // Set ID
                    entry.id = tc.id.clone();
                }
                if let Some(function) = &tc.function {
                    let f = entry.function.get_or_insert_default();
                    if !function.name.is_empty() {
                        // Set type to "function"
                        entry.r#type = "function".into();
                        // Set function name
                        f.name = function.name.clone();
                    }
                    if !function.arguments.is_empty() {
                        // Append function arguments
                        f.arguments.push_str(&function.arguments);
                    }
                }
                if let Some(custom) = &tc.custom {
                    let c = entry.custom.get_or_insert_default();
                    if !custom.name.is_empty() {
                        // Set type to "custom"
                        entry.r#type = "custom".into();
                        // Set custom name
                        c.name = custom.name.clone();
                    }
                    if !custom.input.is_empty() {
                        // Append custom input
                        c.input.push_str(&custom.input);
                    }
                }
            }
        }
    }
    let tool_calls = tool_calls
        .into_iter()
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    (!tool_calls.is_empty()).then_some(tool_calls)
}

/// Builds [`Content`] from chat completion chunks containing content chunks.
fn merge_content(chunks: &[ChatCompletionChunk]) -> Option<Content> {
    let content_text = chunks
        .iter()
        .filter_map(|chunk| {
            chunk
                .choices
                .first()
                .and_then(|choice| choice.delta.content.clone())
        })
        .collect::<String>();
    (!content_text.is_empty()).then_some(Content::Text(content_text))
}

/// Consumes a detection batch stream, builds responses, and sends them to a response channel.
async fn process_detection_batch_stream(
    trace_id: TraceId,
    completion_state: Arc<CompletionState<ChatCompletionChunk>>,
    mut detection_batch_stream: DetectionBatchStream,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    while let Some(result) = detection_batch_stream.next().await {
        match result {
            Ok((choice_index, chunk, detections)) => {
                let input_end_index = chunk.input_end_index;
                match output_detection_response(&completion_state, choice_index, chunk, detections)
                    .await
                {
                    Ok(chat_completion) => {
                        // Send chat completion to response channel
                        debug!(%trace_id, %choice_index, ?chat_completion, "sending chat completion chunk to response channel");
                        if response_tx.send(Ok(Some(chat_completion))).await.is_err() {
                            info!(%trace_id, "task completed: client disconnected");
                            return;
                        }
                        // If this is the final chat completion chunk with content, send chat completion chunk with finish reason
                        // Wait for entry to exist (yields to other tasks until ready)
                        let chat_completions = {
                            let wait_for_entry = async {
                                loop {
                                    if let Some(entry) =
                                        completion_state.completions.get(&choice_index)
                                    {
                                        return entry;
                                    }
                                    tokio::task::yield_now().await;
                                }
                            };

                            match tokio::time::timeout(COMPLETION_WAIT_TIMEOUT, wait_for_entry)
                                .await
                            {
                                Ok(entry) => entry,
                                Err(_) => {
                                    error!(%trace_id, %choice_index, "completion entry not ready after {:?} timeout", COMPLETION_WAIT_TIMEOUT);
                                    return;
                                }
                            }
                        };
                        if chat_completions.keys().rev().nth(1) == Some(&input_end_index)
                            && let Some((_, chat_completion)) = chat_completions.last_key_value()
                            && chat_completion
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
                    Err(error) => {
                        error!(%trace_id, %error, "task failed: error building output detection response");
                        // Send error to response channel and terminate
                        let _ = response_tx.send(Err(error)).await;
                        // Send None to signal completion
                        let _ = response_tx.send(Ok(None)).await;
                        return;
                    }
                }
            }
            Err(error) => {
                error!(%trace_id, %error, "task failed: error received from detection batch stream");
                // Send error to response channel and terminate
                let _ = response_tx.send(Err(error)).await;
                // Send None to signal completion
                let _ = response_tx.send(Ok(None)).await;
                return;
            }
        }
    }
    info!(%trace_id, "task completed: detection batch stream closed");
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_merge_tool_calls() {
        let chunks_json = serde_json::json!([
            {"id":"chatcmpl-d20f671e9f9546858ed53274bd45836e","object":"chat.completion.chunk","created":1759439369,"model":"example","choices":[{"index":0,"delta":{"role":"assistant","content":""},"logprobs":null,"finish_reason":null}]},
            {"id":"chatcmpl-d20f671e9f9546858ed53274bd45836e","object":"chat.completion.chunk","created":1759439369,"model":"example","choices":[{"index":0,"delta":{"tool_calls":[{"id":"chatcmpl-tool-17c2d16c3c734bd69235c88771175bf4","type":"function","index":0,"function":{"name":"get_current_weather"}}]},"logprobs":null,"finish_reason":null}]},
            {"id":"chatcmpl-d20f671e9f9546858ed53274bd45836e","object":"chat.completion.chunk","created":1759439369,"model":"example","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"location\": \""}}]},"logprobs":null,"finish_reason":null}]},
            {"id":"chatcmpl-d20f671e9f9546858ed53274bd45836e","object":"chat.completion.chunk","created":1759439369,"model":"example","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"Boston"}}]},"logprobs":null,"finish_reason":null}]},
            {"id":"chatcmpl-d20f671e9f9546858ed53274bd45836e","object":"chat.completion.chunk","created":1759439369,"model":"example","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":","}}]},"logprobs":null,"finish_reason":null}]},
            {"id":"chatcmpl-d20f671e9f9546858ed53274bd45836e","object":"chat.completion.chunk","created":1759439369,"model":"example","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" MA\""}}]},"logprobs":null,"finish_reason":null}]},
            {"id":"chatcmpl-d20f671e9f9546858ed53274bd45836e","object":"chat.completion.chunk","created":1759439369,"model":"example","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":", \"unit\": \""}}]},"logprobs":null,"finish_reason":null}]},
            {"id":"chatcmpl-d20f671e9f9546858ed53274bd45836e","object":"chat.completion.chunk","created":1759439369,"model":"example","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"f"}}]},"logprobs":null,"finish_reason":null}]},
            {"id":"chatcmpl-d20f671e9f9546858ed53274bd45836e","object":"chat.completion.chunk","created":1759439369,"model":"example","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ahrenheit\"}"}}]},"logprobs":null,"finish_reason":null}]},
            {"id":"chatcmpl-d20f671e9f9546858ed53274bd45836e","object":"chat.completion.chunk","created":1759439369,"model":"example","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":""}}]},"logprobs":null,"finish_reason":"tool_calls","stop_reason":128008}]},
        ]);
        let chunks = serde_json::from_value::<Vec<ChatCompletionChunk>>(chunks_json).unwrap();
        let tool_calls = merge_tool_calls(&chunks);
        assert_eq!(
            tool_calls,
            Some(vec![ToolCall {
                index: Some(0),
                id: "chatcmpl-tool-17c2d16c3c734bd69235c88771175bf4".into(),
                r#type: "function".into(),
                function: Some(Function {
                    name: "get_current_weather".into(),
                    arguments: "{\"location\": \"Boston, MA\", \"unit\": \"fahrenheit\"}".into(),
                }),
                custom: None,
            }])
        );
    }
}
