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

use aggregator::Aggregator;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::future::try_join_all;
use futures::stream::Peekable;
use futures::Stream;
use futures::StreamExt;
use futures::TryStreamExt;
use hyper::HeaderMap;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::ReceiverStream;
use tracing::instrument;
use tracing::{debug, error, info, warn};

use super::streaming::Detections;
use super::Context;
use super::{Error, Orchestrator, StreamingContentDetectionTask};
use crate::clients::chunker::tokenize_whole_doc_stream;
use crate::clients::chunker::ChunkerClient;
use crate::clients::chunker::DEFAULT_CHUNKER_ID;
use crate::clients::detector::ContentAnalysisRequest;
use crate::clients::TextContentsDetectorClient;
use crate::models::DetectorParams;
use crate::models::StreamingContentDetectionRequest;
use crate::models::StreamingContentDetectionResponse;
use crate::models::TokenClassificationResult;
use crate::orchestrator::get_chunker_ids;
use crate::orchestrator::streaming::Chunk;
use crate::pb::caikit::runtime::chunkers;

type ContentInputStream =
    Pin<Box<dyn Stream<Item = Result<StreamingContentDetectionRequest, Error>> + Send>>;

impl Orchestrator {
    /// Handles content detection streaming tasks.
    #[instrument(skip_all, fields(trace_id = task.trace_id.to_string(), headers = ?task.headers))]
    pub async fn handle_streaming_content_detection(
        &self,
        task: StreamingContentDetectionTask,
    ) -> ReceiverStream<Result<StreamingContentDetectionResponse, Error>> {
        let ctx = self.ctx.clone();
        let trace_id = task.trace_id;
        let headers = task.headers;

        let mut input_stream = Box::pin(task.input_stream.peekable());
        let mut _processed_index = 0;

        // Create response channel
        #[allow(clippy::type_complexity)]
        let (response_tx, response_rx): (
            mpsc::Sender<Result<StreamingContentDetectionResponse, Error>>,
            mpsc::Receiver<Result<StreamingContentDetectionResponse, Error>>,
        ) = mpsc::channel(32);

        // Spawn task to process input stream
        tokio::spawn(async move {
            let detectors = match extract_detectors(&mut input_stream).await {
                Ok(detectors) => detectors,
                Err(error) => {
                    error!("{:#?}", error);
                    let _ = response_tx.send(Err(error)).await;
                    return;
                }
            };

            // TODO: figure out a way not to need this bool
            let mut _first_frame = true;

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
                &detectors,
                input_stream,
                error_tx.clone(),
                headers.clone(),
            )
            .await
            {
                Ok(result_rx) => result_rx,
                Err(error) => {
                    error!(%trace_id, %error, "task failed");
                    let _ = error_tx.send(error.clone());
                    let _ = response_tx.send(Err(error)).await;
                    return;
                }
            };
            tokio::spawn(async move {
                let mut error_rx = error_tx.subscribe();
                loop {
                    tokio::select! {
                        Ok(error) = error_rx.recv() => {
                            error!(%trace_id, %error, "task failed");
                            debug!(%trace_id, "sending error to client and terminating");
                            let _ = response_tx.send(Err(error)).await;
                            return;
                        },
                        result = result_rx.recv() => {
                            match result {
                                Some(result) => {
                                    debug!(%trace_id, ?result, "sending result to client");
                                    if (response_tx.send(result).await).is_err() {
                                        warn!(%trace_id, "response channel closed (client disconnected), terminating task");
                                        // Broadcast cancellation signal to tasks
                                        let _ = error_tx.send(Error::Cancelled);
                                        return;
                                    }
                                },
                                None => {
                                    info!(%trace_id, "task completed: stream closed");
                                    break;
                                },
                            }
                        }
                    }
                }
            });

            //     // Process the input stream
            //     while let Some(result) = input_stream.next().await {
            //         match result {
            //             Ok(msg) => {
            //                 // Validation for second input stream frame onward
            //                 if !first_frame {
            //                     if let Err(error) = msg.validate_subsequent_requests() {
            //                         error!("{:#?}", error);
            //                         let _ = response_tx
            //                             .send(Err(Error::Validation(error.to_string())))
            //                             .await;
            //                         return;
            //                     }
            //                 } else {
            //                     first_frame = false;
            //                 }

            //                 // TODO: actual processing
            //                 // Send a dummy response for now
            //                 let start_index = processed_index;
            //                 processed_index += msg.content.len();

            //                 // let mut result_rx = match streaming_output_detection_task(
            //                 //     &ctx,
            //                 //     &detectors,
            //                 //     input_stream.into_inner(),
            //                 //     error_tx.clone(),
            //                 //     headers.clone(),
            //                 // )
            //                 // .await
            //                 // {
            //                 //     Ok(result_rx) => result_rx,
            //                 //     Err(error) => {
            //                 //         error!(%trace_id, %error, "task failed");
            //                 //         let _ = error_tx.send(error.clone());
            //                 //         let _ = respostreaming_content_detection

            //                 let _ = response_tx
            //                     .send(Ok(StreamingContentDetectionResponse {
            //                         detections: Vec::new(),
            //                         processed_index: processed_index as u32,
            //                         start_index: start_index as u32,
            //                     }))
            //                     .await;
            //             }
            //             Err(error) => {
            //                 // json deserialization error, send error message and terminate task
            //                 error!("{:#?}", error);
            //                 let _ = response_tx
            //                     .send(Err(Error::Validation(error.to_string())))
            //                     .await;
            //                 return;
            //             }
            //         }
            //     }
        });
        ReceiverStream::new(response_rx)
    }
}

/// Validates first request frame and returns detectors configuration.
async fn extract_detectors(
    input_stream: &mut Peekable<ContentInputStream>,
) -> Result<HashMap<String, DetectorParams>, Error> {
    // Get detector config from the first message
    // We can use Peekable to get a reference to it instead of consuming the message here
    // Peekable::peek() takes self: Pin<&mut Peekable<_>>, which is why we need to pin it
    // https://docs.rs/futures/latest/futures/stream/struct.Peekable.html
    if let Some(result) = Pin::new(input_stream).peek().await {
        match result {
            Ok(msg) => {
                // validate initial stream frame
                if let Err(error) = msg.validate_initial_request() {
                    error!("{:#?}", error);
                    return Err(Error::Validation(error.to_string()));
                }

                // validate_initial_request() already asserts that `detectors` field exist.
                Ok(msg.detectors.clone().unwrap())
            }
            Err(error) => {
                // json deserialization error, send error message and terminate task
                Err(Error::Validation(error.to_string()))
            }
        }
    } else {
        // TODO: Is this the proper error here?
        let error = Error::Other("Error on extract_detectors outer else".to_string());
        Err(error)
    }
}

/// Handles streaming output detection task.
#[instrument(skip_all)]
async fn streaming_output_detection_task(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    generation_stream: ContentInputStream,
    error_tx: broadcast::Sender<Error>,
    headers: HeaderMap,
) -> Result<mpsc::Receiver<Result<StreamingContentDetectionResponse, Error>>, Error> {
    debug!(?detectors, "creating chunk broadcast streams");

    // Create generation broadcast stream
    let (generation_tx, generation_rx) = broadcast::channel(1024);

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
        // Create a mutable copy of the parameters, so that we can modify it based on processing
        let mut detector_params = detector_params.clone();
        let detector_id = detector_id.to_string();
        let chunker_id = ctx
            .config
            .get_chunker_id(&detector_id)
            .expect("chunker id is not found");

        // Get the detector config
        // TODO: Add error handling
        let detector_config = ctx
            .config
            .detectors
            .get(&detector_id)
            .expect("detector config not found");

        // Get the default threshold to use if threshold is not provided by the user
        let default_threshold = detector_config.default_threshold;
        let threshold = detector_params.pop_threshold().unwrap_or(default_threshold);

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
            detector_params,
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

/// Opens bi-directional stream to a chunker service
/// with generation stream input and returns chunk broadcast stream.
#[instrument(skip_all, fields(chunker_id))]
async fn chunk_broadcast_task(
    ctx: Arc<Context>,
    chunker_id: String,
    generation_rx: broadcast::Receiver<StreamingContentDetectionRequest>,
    error_tx: broadcast::Sender<Error>,
) -> Result<broadcast::Sender<Chunk>, Error> {
    // Consume generation stream and convert to chunker input stream
    debug!("creating chunker input stream");
    // NOTE: Text gen providers can return more than 1 token in single stream object. This can create
    // edge cases where the enumeration generated below may not line up with token / response boundaries.
    // So the more accurate way here might be to use `Tokens` object from response, but since that is an
    // optional response parameter, we are avoiding that for now.
    let input_stream = BroadcastStream::new(generation_rx)
        .enumerate()
        .map(|(token_pointer, generation_result)| {
            let generated_text = generation_result.unwrap().content;
            chunkers::BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: generated_text,
                input_index_stream: token_pointer as i64,
            }
        })
        .boxed();
    debug!("creating chunker output stream");
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
    debug!("spawning chunker broadcast task");
    let (chunk_tx, _) = broadcast::channel(1024);
    tokio::spawn({
        let mut error_rx = error_tx.subscribe();
        let chunk_tx = chunk_tx.clone();
        async move {
            loop {
                tokio::select! {
                    _ = error_rx.recv() => {
                        warn!("cancellation signal received, terminating task");
                        break
                    },
                    result = output_stream.next() => {
                        match result {
                            Some(Ok(chunk)) => {
                                debug!(?chunk, "received chunk");
                                let _ = chunk_tx.send(chunk);
                            },
                            Some(Err(error)) => {
                                error!(%error, "chunker error, cancelling task");
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
    });
    Ok(chunk_tx)
}

/// Wraps a unary detector service to make it streaming.
/// Consumes chunk broadcast stream, sends unary requests to a detector service,
/// and sends chunk + responses to detection stream.
#[allow(clippy::too_many_arguments)]
#[instrument(skip_all, fields(detector_id))]
async fn detection_task(
    ctx: Arc<Context>,
    detector_id: String,
    detector_params: DetectorParams,
    threshold: f64,
    detector_tx: mpsc::Sender<(Chunk, Detections)>,
    mut chunk_rx: broadcast::Receiver<Chunk>,
    error_tx: broadcast::Sender<Error>,
    headers: HeaderMap,
) {
    debug!(threshold, "starting task");
    let mut error_rx = error_tx.subscribe();

    loop {
        tokio::select! {
            _ = error_rx.recv() => {
                warn!("cancellation signal received, terminating task");
                break
            },
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
                            let request = ContentAnalysisRequest::new(contents.clone(), detector_params.clone());
                            let headers = headers.clone();
                            debug!(%detector_id, ?request, "sending detector request");
                            let client = ctx
                                .clients
                                .get_as::<TextContentsDetectorClient>(&detector_id)
                                .unwrap_or_else(|| panic!("text contents detector client not found for {}", detector_id));
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

#[instrument(skip_all)]
async fn generation_broadcast_task(
    mut generation_stream: Pin<
        Box<dyn Stream<Item = Result<StreamingContentDetectionRequest, Error>> + Send>,
    >,
    generation_tx: broadcast::Sender<StreamingContentDetectionRequest>,
    error_tx: broadcast::Sender<Error>,
) {
    debug!("forwarding response stream");
    let mut error_rx = error_tx.subscribe();
    loop {
        tokio::select! {
            _ = error_rx.recv() => {
                warn!("cancellation signal received, terminating task");
                break
            },
            result = generation_stream.next() => {
                match result {
                    Some(Ok(generation)) => {
                        debug!(?generation, "received input request frame");
                        let _ = generation_tx.send(generation);
                    },
                    Some(Err(error)) => {
                        error!(%error, "error on input stream, cancelling task");
                        let _ = error_tx.send(Error::Validation(error.to_string()));
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
