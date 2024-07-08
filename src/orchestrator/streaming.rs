use std::{
    collections::HashMap,
    pin::Pin,
    sync::{Arc, RwLock},
};

use futures::{future::join_all, Stream, StreamExt};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tracing::{debug, info};

use super::{get_chunker_ids, Context, Error, Orchestrator, StreamingClassificationWithGenTask};
use crate::{
    clients::detector::{ContentAnalysisRequest, ContentAnalysisResponse},
    models::{
        ClassifiedGeneratedTextStreamResult, DetectorParams, GuardrailsTextGenerationParameters,
        InputWarning, InputWarningReason, TextGenTokenClassificationResults,
    },
    orchestrator::{
        aggregators::{DetectionAggregator, MaxProcessedIndexAggregator},
        unary::{input_detection_task, tokenize},
        UNSUITABLE_INPUT_MESSAGE,
    },
    pb::{caikit::runtime::chunkers, caikit_data_model::nlp::TokenizationStreamResult},
};

impl Orchestrator {
    /// Handles streaming tasks.
    pub async fn handle_streaming_classification_with_gen(
        &self,
        task: StreamingClassificationWithGenTask,
    ) -> Result<ReceiverStream<ClassifiedGeneratedTextStreamResult>, Error> {
        info!(
            request_id = ?task.request_id,
            model_id = %task.model_id,
            config = ?task.guardrails_config,
            "handling streaming task"
        );

        // TODO:
        // - Improve error handling for streaming
        // - Figure out good approach to share generation messages with detection processors (using shared vec for now)

        let ctx = self.ctx.clone();
        let model_id = task.model_id;
        let params = task.text_gen_parameters;
        let input_text = task.inputs;

        // Create response channel
        let (response_tx, response_rx) = mpsc::channel(1024);

        // Do input detections (unary)
        let masks = task.guardrails_config.input_masks();
        let input_detectors = task.guardrails_config.input_detectors();
        let input_detections = match input_detectors {
            Some(detectors) if !detectors.is_empty() => {
                input_detection_task(&ctx, detectors, input_text.clone(), masks).await?
            }
            _ => None,
        };
        debug!(?input_detections);
        if input_detections.is_some() {
            // Detected HAP/PII
            // Do tokenization to get input_token_count
            let (input_token_count, _tokens) =
                tokenize(&ctx, model_id.clone(), input_text.clone()).await?;
            // Send result with input detections
            let _ = response_tx
                .send(ClassifiedGeneratedTextStreamResult {
                    input_token_count,
                    token_classification_results: TextGenTokenClassificationResults {
                        input: input_detections,
                        output: None,
                    },
                    warnings: Some(vec![InputWarning {
                        id: Some(InputWarningReason::UnsuitableInput),
                        message: Some(UNSUITABLE_INPUT_MESSAGE.to_string()),
                    }]),
                    ..Default::default()
                })
                .await;
        } else {
            // No HAP/PII detected
            // Do text generation (streaming)
            let mut generation_stream =
                generate_stream(&ctx, model_id.clone(), input_text.clone(), params.clone()).await?;

            // Do output detections (streaming)
            let output_detectors = task.guardrails_config.output_detectors();
            match output_detectors {
                Some(detectors) if !detectors.is_empty() => {
                    let aggregator = MaxProcessedIndexAggregator::default();
                    let mut result_rx = streaming_output_detection_task(
                        &ctx,
                        detectors,
                        aggregator,
                        generation_stream,
                    )
                    .await?;
                    // Forward generation results with detections to response channel
                    tokio::spawn(async move {
                        while let Some(generation_with_detections) = result_rx.recv().await {
                            let _ = response_tx.send(generation_with_detections).await;
                        }
                    });
                }
                _ => {
                    // No output detectors, forward generation results to response channel
                    tokio::spawn(async move {
                        while let Some(generation) = generation_stream.next().await {
                            let _ = response_tx.send(generation).await;
                        }
                    });
                }
            }
        }
        Ok(ReceiverStream::new(response_rx))
    }
}

/// Handles streaming output detection task.
async fn streaming_output_detection_task(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    aggregator: impl DetectionAggregator,
    mut generation_stream: Pin<Box<dyn Stream<Item = ClassifiedGeneratedTextStreamResult> + Send>>,
) -> Result<mpsc::Receiver<ClassifiedGeneratedTextStreamResult>, Error> {
    // Create generation broadcast stream
    let (generation_tx, generation_rx) = broadcast::channel(1024);

    debug!("creating chunk broadcast streams");
    let chunker_ids = get_chunker_ids(ctx, detectors)?;

    // Create a map of chunker_id->chunk_broadcast_stream
    // This is to enable fan-out of chunk streams to potentially multiple detectors that use the same chunker.
    // Each detector task will subscribe to an associated chunk stream.
    let chunk_broadcast_streams = join_all(
        chunker_ids
            .into_iter()
            .map(|chunker_id| {
                debug!(%chunker_id, "creating chunk broadcast stream");
                let ctx = ctx.clone();
                // Subscribe to generation stream
                let generation_rx = generation_tx.subscribe();
                async move {
                    let (chunk_tx, chunk_rx) =
                        chunk_broadcast_stream(ctx, chunker_id.clone(), generation_rx)
                            .await
                            .unwrap();
                    (chunker_id, (chunk_tx, chunk_rx))
                }
            })
            .collect::<Vec<_>>(),
    )
    .await
    .into_iter()
    .collect::<HashMap<_, _>>();

    // Spawn detection tasks to subscribe to chunker stream,
    // send requests to detector service, and send results to detection stream
    debug!("spawning detection tasks");
    let mut detection_streams = Vec::with_capacity(detectors.len());
    for detector_id in detectors.keys() {
        let detector_id = detector_id.to_string();
        let chunker_id = ctx.config.get_chunker_id(&detector_id).unwrap();
        // Create detection stream
        let (detector_tx, detector_rx) = mpsc::channel(1024);
        // Subscribe to chunk broadcast stream
        let chunk_rx = chunk_broadcast_streams
            .get(&chunker_id)
            .unwrap()
            .0
            .subscribe();
        tokio::spawn(streaming_detection_task(
            ctx.clone(),
            detector_id.clone(),
            detector_tx,
            chunk_rx,
        ));
        detection_streams.push((detector_id, detector_rx));
    }

    debug!("spawning generation broadcast task");
    // NOTE: this creates a shared vec for detection processors to get details from
    // generation messages. There is probably a better approach.
    let generations = Arc::new(RwLock::new(Vec::new()));

    // Spawn task to consume generation stream and forward to broadcast stream
    tokio::spawn({
        let generations = generations.clone();
        let generation_tx = generation_tx.clone();
        async move {
            while let Some(generation) = generation_stream.next().await {
                debug!(
                    ?generation,
                    "[generation_broadcast_task] received generation"
                );
                // Add a copy to the shared vec
                generations.write().unwrap().push(generation.clone());
                let _ = generation_tx.send(generation);
            }
        }
    });
    drop(generation_tx);
    drop(generation_rx);

    // Process detection results
    Ok(aggregator.process(generations, detection_streams).await)
}

/// This task essentially wraps a unary detector service to make it streaming.
/// Consumes chunk broadcast stream, sends unary requests to a detector service,
/// and sends chunk + responses to detection stream.
async fn streaming_detection_task(
    ctx: Arc<Context>,
    detector_id: String,
    detector_tx: mpsc::Sender<DetectionResult>,
    mut chunk_rx: broadcast::Receiver<TokenizationStreamResult>,
) {
    // Process chunks
    while let Ok(chunk) = chunk_rx.recv().await {
        debug!(%detector_id, ?chunk, "[detection_task] received chunk");
        // Send request to detector service
        let contents = chunk
            .results
            .iter()
            .map(|token| token.text.clone())
            .collect::<Vec<_>>();
        let request = ContentAnalysisRequest::new(contents);
        debug!(%detector_id, ?request, "[detection_task] sending detector request");
        let response = ctx
            .detector_client
            .text_contents(&detector_id, request)
            .await
            .map_err(|error| Error::DetectorRequestFailed {
                detector_id: detector_id.clone(),
                error,
            })
            .unwrap();
        debug!(%detector_id, ?response, "[detection_task] received detector response");
        let result = DetectionResult::new(chunk, response);
        debug!(%detector_id, ?result, "[detection_task] sending result to detector channel");
        let _ = detector_tx.send(result).await;
    }
}

/// Sends generate stream request to a generation service.
async fn generate_stream(
    ctx: &Arc<Context>,
    model_id: String,
    text: String,
    params: Option<GuardrailsTextGenerationParameters>,
) -> Result<Pin<Box<dyn Stream<Item = ClassifiedGeneratedTextStreamResult> + Send>>, Error> {
    ctx.generation_client
        .generate_stream(model_id.clone(), text, params)
        .await
        .map_err(|error| Error::GenerateRequestFailed {
            model_id: model_id.clone(),
            error,
        })
}

/// Opens bi-directional stream to a chunker service
/// with generation stream input and returns chunk broadcast stream.
async fn chunk_broadcast_stream(
    ctx: Arc<Context>,
    chunker_id: String,
    generation_rx: broadcast::Receiver<ClassifiedGeneratedTextStreamResult>,
) -> Result<
    (
        broadcast::Sender<TokenizationStreamResult>,
        broadcast::Receiver<TokenizationStreamResult>,
    ),
    Error,
> {
    // Consume generation stream and convert to chunker input stream
    debug!(%chunker_id, "creating chunker input stream");
    let input_stream = BroadcastStream::new(generation_rx)
        .map(|generation_result| {
            let generated_text = generation_result
                .unwrap()
                .generated_text
                .unwrap_or_default();
            chunkers::BidiStreamingTokenizationTaskRequest {
                text_stream: generated_text,
            }
        })
        .boxed();
    debug!(%chunker_id, "creating chunker output stream");
    let mut output_stream = ctx
        .chunker_client
        .bidi_streaming_tokenization_task_predict(&chunker_id, input_stream)
        .await
        .map_err(|error| Error::ChunkerRequestFailed {
            chunker_id: chunker_id.clone(),
            error,
        })?;

    // Spawn task to consume output stream forward to broadcast channel
    debug!(%chunker_id, "spawning chunker broadcast task");
    let (chunk_tx, chunk_rx) = broadcast::channel(1024);
    tokio::spawn({
        let chunk_tx = chunk_tx.clone();
        async move {
            while let Some(chunk) = output_stream.next().await {
                debug!(%chunker_id, ?chunk, "[chunker_broadcast_task] received chunk");
                let _ = chunk_tx.send(chunk);
            }
        }
    });
    Ok((chunk_tx, chunk_rx))
}

#[derive(Debug, Clone)]
pub struct DetectionResult {
    pub chunk: TokenizationStreamResult,
    pub detections: Vec<Vec<ContentAnalysisResponse>>,
}

impl DetectionResult {
    pub fn new(
        chunk: TokenizationStreamResult,
        detections: Vec<Vec<ContentAnalysisResponse>>,
    ) -> Self {
        Self { chunk, detections }
    }
}

#[cfg(test)]
mod tests {}
