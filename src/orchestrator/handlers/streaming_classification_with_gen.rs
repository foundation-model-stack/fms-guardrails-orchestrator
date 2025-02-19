use std::sync::RwLock;

use futures::StreamExt;

use super::prelude::*;
use crate::orchestrator::types::detection_batch_stream::{DetectionBatch, SimpleBatcher};

impl Handle<StreamingClassificationWithGenTask> for Orchestrator {
    type Response = ReceiverStream<Result<ClassifiedGeneratedTextStreamResult, Error>>;

    async fn handle(
        &self,
        task: StreamingClassificationWithGenTask,
    ) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();
        let headers = task.headers;
        let trace_id = task.trace_id;
        let input_text = task.inputs;
        let model_id = task.model_id;
        let generate_params = task.text_gen_parameters;

        let (response_tx, response_rx) =
            mpsc::channel::<Result<ClassifiedGeneratedTextStreamResult, Error>>(32);

        // Validate guardrails config
        // Return error for unsupported detectors or whole doc chunker
        // validate_guardrails(&ctx, &guardrails)?;

        tokio::spawn(async move {
            let guardrails = task.guardrails_config;
            let input_detectors = guardrails.input_detectors();
            let output_detectors = guardrails.output_detectors();

            // Process input detections (unary)
            let input_detections = if let Some(detectors) = input_detectors {
                let inputs = common::apply_masks(input_text.clone(), guardrails.input_masks());
                match common::text_contents_detections(
                    ctx.clone(),
                    headers.clone(),
                    detectors,
                    inputs,
                )
                .await
                {
                    Ok(detections) => {
                        let detections = detections
                            .into_iter()
                            .map(|(_detector_id, detections)| detections)
                            .collect::<Vec<_>>();
                        (!detections.is_empty()).then_some(detections)
                    }
                    Err(error) => {
                        error!(%trace_id, %error, "task failed: error processing input detections");
                        // Send error to response channel and terminate
                        let _ = response_tx.send(Err(error)).await;
                        return;
                    }
                }
            } else {
                None
            };

            if let Some(detections) = input_detections {
                // Build response message with input detections
                // Get token count
                let input_token_count = match common::tokenize(
                    ctx.clone(),
                    headers.clone(),
                    model_id.clone(),
                    input_text.clone(),
                )
                .await
                {
                    Ok((token_count, _tokens)) => token_count,
                    Err(error) => {
                        error!(%trace_id, %error, "task failed: error tokenizing input text");
                        // Send error to response channel and terminate
                        let _ = response_tx.send(Err(error)).await;
                        return;
                    }
                };
                let response = ClassifiedGeneratedTextStreamResult {
                    input_token_count,
                    token_classification_results: TextGenTokenClassificationResults {
                        // input: Some(detections.into()), // TODO: convert Vec<Detection> into Vec<TokenClassificationResult>
                        input: None,
                        output: None,
                    },
                    warnings: Some(vec![DetectionWarning::unsuitable_input()]),
                    ..Default::default()
                };
                // Send message with input detections to response channel
                let _ = response_tx.send(Ok(response)).await;
            } else {
                // No input detections

                // Create generation stream
                let mut generation_stream = match common::generate_stream(
                    ctx.clone(),
                    headers.clone(),
                    model_id.clone(),
                    input_text.clone(),
                    generate_params.clone(),
                )
                .await
                {
                    Ok(stream) => stream,
                    Err(error) => {
                        error!(%trace_id, %error, "task failed: error creating generation stream");
                        // Send error to response channel and terminate
                        let _ = response_tx.send(Err(error)).await;
                        return;
                    }
                };

                if let Some(detectors) = output_detectors {
                    // Process output detections (streaming)

                    // Create generation broadcast channel
                    let generation_broadcast_tx = common::broadcast_stream(generation_stream);

                    // Create shared generations
                    let generations: Arc<RwLock<Vec<ClassifiedGeneratedTextStreamResult>>> =
                        Arc::new(RwLock::new(Vec::new()));

                    // Spawn task to consume generations and update shared generations
                    tokio::spawn({
                        let generations = generations.clone();
                        let mut generation_rx = generation_broadcast_tx.subscribe();
                        async move {
                            while let Ok(Ok((_index, message))) = generation_rx.recv().await {
                                generations.write().unwrap().push(message);
                            }
                        }
                    });

                    // Create detection streams
                    match common::text_contents_detection_streams(
                        ctx.clone(),
                        headers.clone(),
                        detectors,
                        generation_broadcast_tx,
                    )
                    .await
                    {
                        Ok(detection_streams) => {
                            // Create detection batch stream
                            let detectors = detectors.keys().cloned().collect::<Vec<_>>();
                            let mut detection_batch_stream = DetectionBatchStream::new(
                                SimpleBatcher::new(detectors),
                                detection_streams,
                            );
                            // Consume detection batch stream
                            while let Some(result) = detection_batch_stream.next().await {
                                match result {
                                    Ok(batch) => {
                                        // Create response for this batch with output detections
                                        let response =
                                            detection_batch_response(&generations, batch).unwrap();
                                        // Send message to response channel
                                        if response_tx.send(Ok(response)).await.is_err() {
                                            warn!(%trace_id, "response channel closed (client disconnect), terminating task");
                                            return;
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
                            debug!(%trace_id, "task completed: detection batch stream closed");
                        }
                        Err(error) => {
                            error!(%trace_id, %error, "task failed: error creating detection streams");
                            // Send error to response channel and terminate
                            let _ = response_tx.send(Err(error)).await;
                        }
                    }
                } else {
                    // No output detectors
                    // Consume generation stream
                    while let Some(result) = generation_stream.next().await {
                        match result {
                            Ok((_index, response)) => {
                                // Send message to response channel
                                if response_tx.send(Ok(response)).await.is_err() {
                                    warn!(%trace_id, "response channel closed (client disconnect), terminating task");
                                    return;
                                }
                            }
                            Err(error) => {
                                error!(%trace_id, %error, "task failed: error received from generation stream");
                                // Send error to response channel and terminate
                                let _ = response_tx.send(Err(error)).await;
                                return;
                            }
                        }
                    }
                    debug!(%trace_id, "task completed: generation stream closed");
                }
            }
        });

        Ok(ReceiverStream::new(response_rx))
    }
}

/// Creates a response message for a batch of output detections.
fn detection_batch_response(
    generations: &Arc<RwLock<Vec<ClassifiedGeneratedTextStreamResult>>>,
    batch: DetectionBatch,
) -> Result<ClassifiedGeneratedTextStreamResult, Error> {
    let DetectionBatch { chunk, detections } = batch;
    // Get subset of generations relevant for this chunk
    let generations_slice = generations
        .read()
        .unwrap()
        .get(chunk.input_start_index..=chunk.input_end_index)
        .unwrap_or_default()
        .to_vec();
    let last = generations_slice.last().cloned().unwrap_or_default();
    let tokens = generations_slice
        .iter()
        .flat_map(|generation| generation.tokens.clone().unwrap_or_default())
        .collect::<Vec<_>>();
    let mut response = ClassifiedGeneratedTextStreamResult {
        generated_text: Some(chunk.text),
        start_index: Some(chunk.start as u32),
        processed_index: Some(chunk.end as u32),
        tokens: Some(tokens),
        // Populate fields from last response or default
        ..generations_slice.last().cloned().unwrap_or_default()
    };
    // response.token_classification_results.output = Some(detections); // TODO
    if chunk.input_start_index == 0 {
        // Get input_token_count and seed from first generation message
        let first = generations_slice.first().unwrap();
        response.input_token_count = first.input_token_count;
        response.seed = first.seed;
        // Get input_tokens from second generation message (if specified)
        response.input_tokens = if let Some(second) = generations_slice.get(1) {
            second.input_tokens.clone()
        } else {
            Some(Vec::default())
        };
    }
    Ok(response)
}

#[derive(Debug)]
pub struct StreamingClassificationWithGenTask {
    pub trace_id: TraceId,
    pub model_id: String,
    pub inputs: String,
    pub guardrails_config: GuardrailsConfig,
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
    pub headers: HeaderMap,
}

impl StreamingClassificationWithGenTask {
    pub fn new(trace_id: TraceId, request: GuardrailsHttpRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
            model_id: request.model_id,
            inputs: request.inputs,
            guardrails_config: request.guardrail_config.unwrap_or_default(),
            text_gen_parameters: request.text_gen_parameters,
            headers,
        }
    }
}
