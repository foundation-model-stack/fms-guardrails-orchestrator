use std::{collections::HashMap, pin::Pin, sync::Arc};

use futures::{
    future::{self, join_all},
    Stream, StreamExt,
};
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
        processors::{DetectionStreamProcessor, MaxProcessedIndexProcessor},
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

        let ctx = self.ctx.clone();
        let model_id = task.model_id;
        let params = task.text_gen_parameters;
        let input_text = task.inputs;

        // Create response channel
        let (response_tx, response_rx) = mpsc::channel(1024);

        // Do input detections (unary)
        let masks = task.guardrails_config.input_masks();
        let input_detectors = task.guardrails_config.input_detectors();
        let input_detections = if let Some(detectors) = input_detectors {
            input_detection_task(&ctx, detectors, input_text.clone(), masks).await?
        } else {
            None
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
            if let Some(detectors) = output_detectors {
                let processor = MaxProcessedIndexProcessor::default();
                let mut result_rx =
                    streaming_output_detection_task(&ctx, detectors, processor, generation_stream)
                        .await?;
                // Forward generation results with detections to response channel
                tokio::spawn(async move {
                    while let Some(generation_with_detections_result) = result_rx.recv().await {
                        let _ = response_tx.send(generation_with_detections_result).await;
                    }
                });
            } else {
                // No output detectors, forward generation results to response channel
                tokio::spawn(async move {
                    while let Some(generation_result) = generation_stream.next().await {
                        let _ = response_tx.send(generation_result).await;
                    }
                });
            }
        }
        Ok(ReceiverStream::new(response_rx))
    }
}

/***************************** Task handlers ******************************/

/// Handles streaming output detection task.
async fn streaming_output_detection_task(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    processor: impl DetectionStreamProcessor,
    mut generation_stream: Pin<Box<dyn Stream<Item = ClassifiedGeneratedTextStreamResult> + Send>>,
) -> Result<mpsc::Receiver<ClassifiedGeneratedTextStreamResult>, Error> {
    // Create generation broadcast stream
    let (generation_tx, generation_rx) = broadcast::channel(1024);

    debug!("creating chunk broadcast streams");
    let chunker_ids = get_chunker_ids(ctx, detectors)?;
    // Maps chunker_id->chunk_stream
    let chunk_streams = join_all(
        chunker_ids
            .into_iter()
            .map(|chunker_id| {
                debug!(%chunker_id, "creating chunk broadcast stream");
                let ctx = ctx.clone();
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
        let chunk_rx = chunk_streams.get(&chunker_id).unwrap().0.subscribe();
        tokio::spawn(streaming_detection_task(
            ctx.clone(),
            detector_id.clone(),
            detector_tx,
            chunk_rx,
        ));
        detection_streams.push((detector_id, detector_rx));
    }

    debug!("spawning generation broadcast task");
    // Spawn task to consume generation stream and forward to broadcast stream
    tokio::spawn({
        let generation_tx = generation_tx.clone();
        async move {
            while let Some(result) = generation_stream.next().await {
                debug!("[generation_broadcast_task] received: {result:?}");
                let _ = generation_tx.send(result);
            }
        }
    });
    drop(generation_tx);
    drop(generation_rx);

    // Process detection results
    let result_rx = processor.process(detection_streams).await;
    Ok(result_rx)
}

async fn streaming_detection_task(
    ctx: Arc<Context>,
    detector_id: String,
    detector_tx: mpsc::Sender<DetectionResult>,
    mut chunk_rx: broadcast::Receiver<TokenizationStreamResult>,
) {
    // Process chunks
    while let Ok(chunk) = chunk_rx.recv().await {
        debug!(%detector_id, "[detection_task] received: {chunk:?}");
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
/************************** Requests to services **************************/

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
        .filter(|generation_result| {
            // NOTE: this filter is temporary until empty string handling fix is implemented in chunker service
            future::ready(
                generation_result
                    .as_ref()
                    .is_ok_and(|r| r.generated_text != Some("".to_string())),
            )
        })
        .map(|generation_result| {
            let generated_text = generation_result
                .unwrap()
                .generated_text
                .unwrap_or_default();
            //trace!("[chunker_input_stream] received: {generated_text}");
            chunkers::BidiStreamingTokenizationTaskRequest {
                text_stream: generated_text,
            }
        });
    // TEMP: add dummy request to init stream to work around issue https://github.com/hyperium/tonic/issues/515
    let init_input_stream =
        futures::stream::iter(vec![chunkers::BidiStreamingTokenizationTaskRequest {
            text_stream: "init message 1. init message 2.".into(),
        }]);
    let input_stream = init_input_stream.chain(input_stream).boxed();
    debug!(%chunker_id, "creating chunker output stream");
    let mut output_stream = ctx
        .chunker_client
        .bidi_streaming_tokenization_task_predict(&chunker_id, input_stream)
        .await
        .map_err(|error| Error::ChunkerRequestFailed {
            chunker_id: chunker_id.clone(),
            error,
        })?;
    // TEMP: consume/discard initial dummy response, although the second one will still be in the stream
    let _ = output_stream.next().await;

    // Spawn task to consume output stream forward to broadcast channel
    debug!(%chunker_id, "spawning chunker broadcast task");
    let (chunk_tx, chunk_rx) = broadcast::channel(1024);
    tokio::spawn({
        let chunk_tx = chunk_tx.clone();
        async move {
            while let Some(chunk_result) = output_stream.next().await {
                debug!(%chunker_id, "[chunker_broadcast_task] received: {chunk_result:?}");
                if let Ok(chunk_result) = chunk_result {
                    let _ = chunk_tx.send(chunk_result);
                }
            }
        }
    });
    Ok((chunk_tx, chunk_rx))
}

#[cfg(test)]
mod tests {}
