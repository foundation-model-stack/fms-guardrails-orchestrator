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

mod aggregator;

use std::{collections::HashMap, pin::Pin, sync::Arc, time::Duration};

use aggregator::Aggregator;
use axum::http::HeaderMap;
use futures::{future::try_join_all, Stream, StreamExt, TryStreamExt};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tracing::{debug, error, info, instrument};

use super::{get_chunker_ids, Context, Error, Orchestrator, StreamingClassificationWithGenTask};
use crate::{
    clients::{
        chunker::{tokenize_whole_doc_stream, ChunkerClient, DEFAULT_CHUNKER_ID},
        detector::ContentAnalysisRequest,
        GenerationClient, TextContentsDetectorClient,
    },
    models::{
        ClassifiedGeneratedTextStreamResult, DetectorParams, GuardrailsTextGenerationParameters,
        InputWarning, InputWarningReason, TextGenTokenClassificationResults,
        TokenClassificationResult,
    },
    orchestrator::{
        unary::{input_detection_task, tokenize},
        UNSUITABLE_INPUT_MESSAGE,
    },
    pb::{caikit::runtime::chunkers, caikit_data_model::nlp::ChunkerTokenizationStreamResult},
};

pub type Chunk = ChunkerTokenizationStreamResult;
pub type Detections = Vec<TokenClassificationResult>;

impl Orchestrator {
    /// Handles streaming tasks.
    #[instrument(name = "stream_handler", skip_all)]
    pub async fn handle_streaming_classification_with_gen(
        &self,
        task: StreamingClassificationWithGenTask,
    ) -> ReceiverStream<Result<ClassifiedGeneratedTextStreamResult, Error>> {
        let ctx = self.ctx.clone();
        let request_id = task.request_id;
        let model_id = task.model_id;
        let params = task.text_gen_parameters;
        let input_text = task.inputs;
        let headers = task.headers;

        info!(%request_id, config = ?task.guardrails_config, "starting task");

        // Create response channel
        #[allow(clippy::type_complexity)]
        let (response_tx, response_rx): (
            mpsc::Sender<Result<ClassifiedGeneratedTextStreamResult, Error>>,
            mpsc::Receiver<Result<ClassifiedGeneratedTextStreamResult, Error>>,
        ) = mpsc::channel(1024);

        tokio::spawn(async move {
            // Do input detections (unary)
            let masks = task.guardrails_config.input_masks();
            let input_detectors = task.guardrails_config.input_detectors();
            let input_detections = match input_detectors {
                Some(detectors) if !detectors.is_empty() => {
                    match input_detection_task(
                        &ctx,
                        detectors,
                        input_text.clone(),
                        masks,
                        headers.clone(),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(error) => {
                            error!(%request_id, %error, "task failed");
                            let _ = response_tx.send(Err(error)).await;
                            return;
                        }
                    }
                }
                _ => None,
            };
            debug!(?input_detections);
            if let Some(mut input_detections) = input_detections {
                // Detected HAP/PII
                // Do tokenization to get input_token_count
                let (input_token_count, _tokens) =
                    match tokenize(&ctx, model_id.clone(), input_text.clone(), headers.clone())
                        .await
                    {
                        Ok(result) => result,
                        Err(error) => {
                            error!(%request_id, %error, "task failed");
                            let _ = response_tx.send(Err(error)).await;
                            return;
                        }
                    };
                input_detections.sort_by_key(|r| r.start);
                // Send result with input detections
                let _ = response_tx
                    .send(Ok(ClassifiedGeneratedTextStreamResult {
                        input_token_count,
                        token_classification_results: TextGenTokenClassificationResults {
                            input: Some(input_detections),
                            output: None,
                        },
                        warnings: Some(vec![InputWarning {
                            id: Some(InputWarningReason::UnsuitableInput),
                            message: Some(UNSUITABLE_INPUT_MESSAGE.to_string()),
                        }]),
                        ..Default::default()
                    }))
                    .await;
            } else {
                // No HAP/PII detected
                // Do text generation (streaming)
                let mut generation_stream = match generate_stream(
                    &ctx,
                    model_id.clone(),
                    input_text.clone(),
                    params.clone(),
                    headers.clone(),
                )
                .await
                {
                    Ok(generation_stream) => generation_stream,
                    Err(error) => {
                        error!(%request_id, %error, "task failed");
                        let _ = response_tx.send(Err(error)).await;
                        return;
                    }
                };

                // Do output detections (streaming)
                let output_detectors = task.guardrails_config.output_detectors();
                match output_detectors {
                    Some(detectors) if !detectors.is_empty() => {
                        // Create error channel
                        //
                        // This channel is used for error notification & messaging and task cancellation.
                        // When a task fails, it notifies other tasks by sending the error to error_tx.
                        //
                        // The parent task receives the error, logs it, forwards it to the client via response_tx,
                        // and terminates the task.
                        let (error_tx, _) = broadcast::channel(1);

                        let mut result_rx = match streaming_output_detection_task(
                            &ctx,
                            detectors,
                            generation_stream,
                            error_tx.clone(),
                            headers.clone(),
                        )
                        .await
                        {
                            Ok(result_rx) => result_rx,
                            Err(error) => {
                                error!(%request_id, %error, "task failed");
                                let _ = error_tx.send(error.clone());
                                let _ = response_tx.send(Err(error)).await;
                                return;
                            }
                        };
                        // Forward generation results with detections to response channel
                        tokio::spawn(async move {
                            let mut error_rx = error_tx.subscribe();
                            loop {
                                tokio::select! {
                                    Ok(error) = error_rx.recv() => {
                                        error!(%request_id, %error, "task failed");
                                        debug!(%request_id, "sending error to client and terminating");
                                        let _ = response_tx.send(Err(error)).await;
                                        return;
                                    },
                                    result = result_rx.recv() => {
                                        match result {
                                            Some(result) => {
                                                debug!(%request_id, ?result, "sending result to client");
                                                let _ = response_tx.send(result).await;
                                            },
                                            None => {
                                                info!(%request_id, "task completed: stream closed");
                                                break;
                                            },
                                        }
                                    }
                                }
                            }
                        });
                    }
                    _ => {
                        // No output detectors, forward generation results to response channel
                        tokio::spawn(async move {
                            while let Some(result) = generation_stream.next().await {
                                debug!(%request_id, ?result, "sending result to client");
                                let _ = response_tx.send(result).await;
                            }
                            debug!(%request_id, "task completed: stream closed");
                        });
                    }
                }
            }
        });
        ReceiverStream::new(response_rx)
    }
}

/// Handles streaming output detection task.
#[instrument(skip_all)]
async fn streaming_output_detection_task(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    generation_stream: Pin<
        Box<dyn Stream<Item = Result<ClassifiedGeneratedTextStreamResult, Error>> + Send>,
    >,
    error_tx: broadcast::Sender<Error>,
    headers: HeaderMap,
) -> Result<mpsc::Receiver<Result<ClassifiedGeneratedTextStreamResult, Error>>, Error> {
    // Create generation broadcast stream
    let (generation_tx, generation_rx) = broadcast::channel(1024);

    debug!("creating chunk broadcast streams");
    let chunker_ids = get_chunker_ids(ctx, detectors)?;
    // Create a map of chunker_id->chunk_broadcast_stream
    // This is to enable fan-out of chunk streams to potentially multiple detectors that use the same chunker.
    // Each detector task will subscribe to an associated chunk stream.
    let chunk_broadcast_streams = try_join_all(
        chunker_ids
            .into_iter()
            .map(|chunker_id| {
                debug!(%chunker_id, "creating chunk broadcast stream");
                let ctx = ctx.clone();
                let error_tx = error_tx.clone();
                // Subscribe to generation stream
                let generation_rx = generation_tx.subscribe();
                async move {
                    let chunk_tx =
                        chunk_broadcast_task(ctx, chunker_id.clone(), generation_rx, error_tx)
                            .await?;
                    Ok::<(String, broadcast::Sender<Chunk>), Error>((chunker_id, chunk_tx))
                }
            })
            .collect::<Vec<_>>(),
    )
    .await?
    .into_iter()
    .collect::<HashMap<_, _>>();

    // Spawn detection tasks to subscribe to chunker stream,
    // send requests to detector service, and send results to detection stream
    debug!("spawning detection tasks");
    let mut detection_streams = Vec::with_capacity(detectors.len());
    for (detector_id, detector_params) in detectors.iter() {
        let detector_id = detector_id.to_string();
        let chunker_id = ctx.config.get_chunker_id(&detector_id).unwrap();

        // Get the detector config
        // TODO: Add error handling
        let detector_config = ctx.config.detectors.get(&detector_id).unwrap();

        // Get the default threshold to use if threshold is not provided by the user
        let default_threshold = detector_config.default_threshold;
        let threshold = detector_params.threshold().unwrap_or(default_threshold);

        // Create detection stream
        let (detector_tx, detector_rx) = mpsc::channel(1024);
        // Subscribe to chunk broadcast stream
        let chunk_rx = chunk_broadcast_streams
            .get(&chunker_id)
            .unwrap()
            .subscribe();
        let error_tx = error_tx.clone();
        tokio::spawn(detection_task(
            ctx.clone(),
            detector_id.clone(),
            threshold,
            detector_tx,
            chunk_rx,
            error_tx,
            headers.clone(),
        ));
        detection_streams.push((detector_id, detector_rx));
    }

    debug!("processing detection streams");
    let aggregator = Aggregator::default();
    let result_rx = aggregator.run(generation_tx.subscribe(), detection_streams);

    debug!("spawning generation broadcast task");
    // Spawn task to consume generation stream and forward to broadcast stream
    tokio::spawn(generation_broadcast_task(
        generation_stream,
        generation_tx,
        error_tx.clone(),
    ));
    drop(generation_rx);

    Ok(result_rx)
}

#[instrument(skip_all)]
async fn generation_broadcast_task(
    mut generation_stream: Pin<
        Box<dyn Stream<Item = Result<ClassifiedGeneratedTextStreamResult, Error>> + Send>,
    >,
    generation_tx: broadcast::Sender<ClassifiedGeneratedTextStreamResult>,
    error_tx: broadcast::Sender<Error>,
) {
    let mut error_rx = error_tx.subscribe();
    loop {
        tokio::select! {
            _ = error_rx.recv() => { break },
            result = generation_stream.next() => {
                match result {
                    Some(Ok(generation)) => {
                        debug!(?generation, "received generation");
                        let _ = generation_tx.send(generation);
                    },
                    Some(Err(error)) => {
                        error!(%error, "generation error, cancelling task");
                        let _ = error_tx.send(error);
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        break;
                    },
                    None => {
                        debug!("stream closed");
                        break
                    },
                }
            }
        }
    }
}

/// Wraps a unary detector service to make it streaming.
/// Consumes chunk broadcast stream, sends unary requests to a detector service,
/// and sends chunk + responses to detection stream.
#[instrument(skip_all)]
async fn detection_task(
    ctx: Arc<Context>,
    detector_id: String,
    threshold: f64,
    detector_tx: mpsc::Sender<(Chunk, Detections)>,
    mut chunk_rx: broadcast::Receiver<Chunk>,
    error_tx: broadcast::Sender<Error>,
    headers: HeaderMap,
) {
    let mut error_rx = error_tx.subscribe();

    loop {
        tokio::select! {
            _ = error_rx.recv() => { break },
            result = chunk_rx.recv() => {
                match result {
                    Ok(chunk) => {
                        debug!(%detector_id, ?chunk, "received chunk");
                        // Send request to detector service
                        let contents = chunk
                            .results
                            .iter()
                            .map(|token| token.text.clone())
                            .collect::<Vec<_>>();
                        if contents.is_empty() {
                            debug!("empty chunk, skipping detector request.");
                            break;
                        } else {
                            let request = ContentAnalysisRequest::new(contents.clone());
                            let headers = headers.clone();
                            debug!(%detector_id, ?request, "sending detector request");
                            let client = ctx
                                .clients
                                .get_as::<TextContentsDetectorClient>(&detector_id)
                                .unwrap();
                            match client.text_contents(&detector_id, request, headers)
                                .await
                                .map_err(|error| Error::DetectorRequestFailed { id: detector_id.clone(), error }) {
                                    Ok(response) => {
                                        debug!(%detector_id, ?response, "received detector response");
                                        let detections = response
                                            .into_iter()
                                            .flat_map(|r| {
                                                r.into_iter().filter_map(|resp| {
                                                    let result: TokenClassificationResult = resp.into();
                                                    (result.score >= threshold).then_some(result)
                                                })
                                            })
                                            .collect::<Vec<_>>();
                                        let _ = detector_tx.send((chunk, detections)).await;
                                    },
                                    Err(error) => {
                                        error!(%detector_id, %error, "detector error, cancelling task");
                                        let _ = error_tx.send(error);
                                        tokio::time::sleep(Duration::from_millis(5)).await;
                                        break;
                                    },
                                }
                        }
                    },
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!(%detector_id, "stream closed");
                        break;
                    },
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        debug!(%detector_id, "stream lagged");
                        continue;
                    }
                }
            },
        }
    }
}

/// Opens bi-directional stream to a chunker service
/// with generation stream input and returns chunk broadcast stream.
#[instrument(skip_all)]
async fn chunk_broadcast_task(
    ctx: Arc<Context>,
    chunker_id: String,
    generation_rx: broadcast::Receiver<ClassifiedGeneratedTextStreamResult>,
    error_tx: broadcast::Sender<Error>,
) -> Result<broadcast::Sender<Chunk>, Error> {
    // Consume generation stream and convert to chunker input stream
    debug!(%chunker_id, "creating chunker input stream");
    // NOTE: Text gen providers can return more than 1 token in single stream object. This can create
    // edge cases where the enumeration generated below may not line up with token / response boundaries.
    // So the more accurate way here might be to use `Tokens` object from response, but since that is an
    // optional response parameter, we are avoiding that for now.
    let input_stream = BroadcastStream::new(generation_rx)
        .enumerate()
        .map(|(token_pointer, generation_result)| {
            let generated_text = generation_result
                .unwrap()
                .generated_text
                .unwrap_or_default();
            chunkers::BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: generated_text,
                input_index_stream: token_pointer as i64,
            }
        })
        .boxed();
    debug!(%chunker_id, "creating chunker output stream");
    let id = chunker_id.clone(); // workaround for StreamExt::map_err

    let response_stream = if chunker_id == DEFAULT_CHUNKER_ID {
        info!("Using default whole doc chunker");
        let (response_tx, response_rx) = mpsc::channel(1);
        // Spawn task to collect input stream
        tokio::spawn(async move {
            // NOTE: this will not resolve until the input stream is closed
            let response = tokenize_whole_doc_stream(input_stream).await;
            let _ = response_tx.send(response).await;
        });
        Ok(ReceiverStream::new(response_rx).boxed())
    } else {
        let client = ctx.clients.get_as::<ChunkerClient>(&chunker_id).unwrap();
        client
            .bidi_streaming_tokenization_task_predict(&chunker_id, input_stream)
            .await
    };

    let mut output_stream = response_stream
        .map_err(|error| Error::ChunkerRequestFailed {
            id: chunker_id.clone(),
            error,
        })?
        .map_err(move |error| Error::ChunkerRequestFailed {
            id: id.clone(),
            error,
        }); // maps stream errors

    // Spawn task to consume output stream forward to broadcast channel
    debug!(%chunker_id, "spawning chunker broadcast task");
    let (chunk_tx, _) = broadcast::channel(1024);
    tokio::spawn({
        let mut error_rx = error_tx.subscribe();
        let chunk_tx = chunk_tx.clone();
        async move {
            loop {
                tokio::select! {
                    _ = error_rx.recv() => { break },
                    result = output_stream.next() => {
                        match result {
                            Some(Ok(chunk)) => {
                                debug!(%chunker_id, ?chunk, "received chunk");
                                let _ = chunk_tx.send(chunk);
                            },
                            Some(Err(error)) => {
                                error!(%chunker_id, %error, "chunker error, cancelling task");
                                let _ = error_tx.send(error);
                                tokio::time::sleep(Duration::from_millis(5)).await;
                                break;
                            },
                            None => {
                                debug!(%chunker_id, "stream closed");
                                break
                            },
                        }
                    }
                }
            }
        }
    });
    Ok(chunk_tx)
}

/// Sends generate stream request to a generation service.
async fn generate_stream(
    ctx: &Arc<Context>,
    model_id: String,
    text: String,
    params: Option<GuardrailsTextGenerationParameters>,
    headers: HeaderMap,
) -> Result<
    Pin<Box<dyn Stream<Item = Result<ClassifiedGeneratedTextStreamResult, Error>> + Send>>,
    Error,
> {
    let client = ctx
        .clients
        .get_as::<GenerationClient>("generation")
        .unwrap();
    Ok(client
        .generate_stream(model_id.clone(), text, params, headers)
        .await
        .map_err(|error| Error::GenerateRequestFailed {
            id: model_id.clone(),
            error,
        })?
        .map_err(move |error| Error::GenerateRequestFailed {
            id: model_id.clone(),
            error,
        }) // maps stream errors
        .boxed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_generation_broadcast_task() {
        let (generation_tx, generation_rx) = mpsc::channel(4);
        let (generation_broadcast_tx, mut generation_broadcast_rx) = broadcast::channel(4);
        let generation_stream = ReceiverStream::new(generation_rx).boxed();
        let (error_tx, _) = broadcast::channel(1);
        let results = vec![
            ClassifiedGeneratedTextStreamResult {
                generated_text: Some("hello".into()),
                ..Default::default()
            },
            ClassifiedGeneratedTextStreamResult {
                generated_text: Some(" ".into()),
                ..Default::default()
            },
            ClassifiedGeneratedTextStreamResult {
                generated_text: Some("world".into()),
                ..Default::default()
            },
        ];
        tokio::spawn({
            let results = results.clone();
            async move {
                for result in results {
                    let _ = generation_tx.send(Ok(result)).await;
                }
            }
        });
        tokio::spawn(generation_broadcast_task(
            generation_stream,
            generation_broadcast_tx,
            error_tx,
        ));
        let mut broadcast_results = Vec::with_capacity(results.len());
        while let Ok(result) = generation_broadcast_rx.recv().await {
            println!("{result:?}");
            broadcast_results.push(result);
        }
        assert_eq!(results, broadcast_results)
    }
}
