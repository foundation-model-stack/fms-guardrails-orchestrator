use std::sync::RwLock;

use futures::StreamExt;
use uuid::Uuid;

use super::super::prelude::*;
use crate::clients::openai::*;

pub async fn handle_streaming(
    ctx: Arc<Context>,
    task: ChatCompletionsDetectionTask,
) -> Result<ChatCompletionsResponse, Error> {
    // Create response channel
    let (response_tx, response_rx) =
        mpsc::channel::<Result<Option<ChatCompletionChunk>, Error>>(32);

    tokio::spawn(async move {
        let trace_id = &task.trace_id;
        let headers = &task.headers;
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
                Ok(Some(completion_chunk)) => {
                    info!(%trace_id, "task completed: returning response with input detections");
                    // Send message with input detections to response channel and terminate
                    let _ = response_tx.send(Ok(Some(completion_chunk))).await;
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
        let request = task.request.clone();
        let chat_completion_stream = match common::chat_completion_stream(
            ctx.clone(),
            headers.clone(),
            request,
        )
        .await
        {
            Ok(stream) => stream,
            Err(error) => {
                error!(%trace_id, %error, "task failed: error creating chat completions stream");
                // Send error to response channel and terminate
                let _ = response_tx.send(Err(error)).await;
                return;
            }
        };

        // Handle output detection
        if let Some(detectors) = output_detectors {
            // Handle output detection
            handle_output_detection(
                ctx.clone(),
                &task,
                &detectors,
                chat_completion_stream,
                response_tx,
            )
            .await;
        } else {
            // No output detectors, forward chat completion stream to response stream
            forward_chat_completion_stream(trace_id, chat_completion_stream, response_tx).await;
        }

        // Handle input-output detection

        todo!()
    });

    Ok(ChatCompletionsResponse::Streaming(response_rx))
}

async fn handle_output_detection(
    ctx: Arc<Context>,
    task: &ChatCompletionsDetectionTask,
    detectors: &HashMap<String, DetectorParams>,
    chat_completion_stream: ChatCompletionStream,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    let trace_id = &task.trace_id;
    let request = task.request.clone();
    let n = request.n.unwrap_or(1);

    // Create input channels
    let mut input_senders = HashMap::with_capacity(n as usize);
    let mut input_receivers = HashMap::with_capacity(n as usize);
    (1..=n).for_each(|input_id| {
        let (input_tx, input_rx) = mpsc::channel::<Result<(usize, String), Error>>(32);
        input_senders.insert(input_id, input_tx);
        input_receivers.insert(input_id, input_rx);
    });

    // Create shared chat completions
    let chat_completions: Arc<RwLock<Vec<ChatCompletionChunk>>> = Arc::new(RwLock::new(Vec::new()));

    // Spawn task to process chat completions
    // Handles sending chunk choices to their respective input channels
    // and updating shared chat completions.
    tokio::spawn(process_chat_completion_stream(
        chat_completions.clone(),
        chat_completion_stream,
        input_senders,
    ));

    // Create detection streams
    let mut detection_streams = Vec::with_capacity(n as usize * detectors.len());
    for (input_id, input_rx) in input_receivers {
        match common::text_contents_detection_streams(
            ctx.clone(),
            task.headers.clone(),
            detectors,
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
        let detectors = detectors.keys().cloned().collect::<Vec<_>>();
        let detection_batch_stream =
            DetectionBatchStream::new(ChatCompletionBatcher::new(detectors), detection_streams);
        process_detection_batch_stream(
            trace_id,
            chat_completions,
            detection_batch_stream,
            response_tx,
        )
        .await;
    }
}

/// Consumes a chat completion stream, forwarding messages to a response channel.
async fn forward_chat_completion_stream(
    trace_id: &TraceId,
    mut chat_completion_stream: ChatCompletionStream,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    while let Some(result) = chat_completion_stream.next().await {
        match result {
            Ok((_index, response)) => {
                // Send message to response channel
                if response_tx.send(Ok(Some(response))).await.is_err() {
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

/// Consumes a detection stream, builds responses, and sends them to a response channel.
async fn process_detection_stream(
    trace_id: &TraceId,
    chat_completions: Arc<RwLock<Vec<ChatCompletionChunk>>>,
    mut detection_stream: DetectionStream,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    while let Some(result) = detection_stream.next().await {
        match result {
            Ok((input_id, _detector_id, chunk, detections)) => {
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
    chat_completions: Arc<RwLock<Vec<ChatCompletionChunk>>>,
    mut detection_batch_stream: DetectionBatchStream<ChatCompletionBatcher>,
    response_tx: mpsc::Sender<Result<Option<ChatCompletionChunk>, Error>>,
) {
    while let Some(result) = detection_batch_stream.next().await {
        match result {
            Ok(batch) => {
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

async fn process_chat_completion_stream(
    chat_completions: Arc<RwLock<Vec<ChatCompletionChunk>>>,
    mut chat_completion_stream: ChatCompletionStream,
    input_senders: HashMap<InputId, InputSender>,
) {
    while let Some(result) = chat_completion_stream.next().await {
        match result {
            Ok((index, completion)) => {
                for choice in &completion.choices {
                    let input_id = choice.index;
                    // Send generated text to input channel
                    let input = (index, choice.clone().delta.content.unwrap_or_default());
                    let input_tx = input_senders.get(&input_id).unwrap();
                    let _ = input_tx.send(Ok(input)).await;
                }
                // Update shared chat completions
                chat_completions.write().unwrap().push(completion);
            }
            Err(error) => {
                // Send error to all input channels
                for input_tx in input_senders.values() {
                    let _ = input_tx.send(Err(error.clone())).await;
                }
            }
        }
    }
}

async fn handle_input_detection(
    ctx: Arc<Context>,
    task: &ChatCompletionsDetectionTask,
    detectors: &HashMap<String, DetectorParams>,
) -> Result<Option<ChatCompletionChunk>, Error> {
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
        // Build response with input detections
        let completion_chunk = input_detection_response(model_id, detections);
        Ok(Some(completion_chunk))
    } else {
        // No input detections
        Ok(None)
    }
}

/// Builds a response with input detections.
fn input_detection_response(model_id: String, detections: Detections) -> ChatCompletionChunk {
    ChatCompletionChunk {
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
