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
use std::{collections::HashMap, pin::Pin, sync::Arc};

use futures::{Stream, StreamExt, stream::Peekable};
use http::HeaderMap;
use opentelemetry::trace::TraceId;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{Instrument, error, info, instrument};

use super::Handle;
use crate::{
    config::DetectorType,
    models::{DetectorParams, StreamingContentDetectionRequest, StreamingContentDetectionResponse},
    orchestrator::{
        Context, Error, Orchestrator,
        common::{self, validate_detectors},
        types::{BoxStream, DetectionBatchStream, MaxProcessedIndexBatcher},
    },
};

type InputStream =
    Pin<Box<dyn Stream<Item = (usize, Result<StreamingContentDetectionRequest, Error>)> + Send>>;

impl Handle<StreamingContentDetectionTask> for Orchestrator {
    type Response = ReceiverStream<Result<StreamingContentDetectionResponse, Error>>;

    #[instrument(
        name = "streaming_content_detection",
        skip_all,
        fields(trace_id = task.trace_id.to_string(), headers = ?task.headers)
    )]
    async fn handle(&self, task: StreamingContentDetectionTask) -> Result<Self::Response, Error> {
        let ctx = self.ctx.clone();

        // Create response channel
        let (response_tx, response_rx) =
            mpsc::channel::<Result<StreamingContentDetectionResponse, Error>>(128);

        tokio::spawn(
            async move {
                let trace_id = task.trace_id;
                let headers = task.headers;
                let mut input_stream = Box::pin(task.input_stream.peekable());
                let detectors = match extract_detectors(&mut input_stream).await {
                    Ok(detectors) => detectors,
                    Err(error) => {
                        error!(%error, "error extracting detectors from first message");
                        let _ = response_tx.send(Err(error)).await;
                        return;
                    }
                };
                info!(%trace_id, config = ?detectors, "task started");

                if let Err(error) = validate_detectors(
                    &detectors,
                    &ctx.config.detectors,
                    &[DetectorType::TextContents],
                    false,
                ) {
                    let _ = response_tx.send(Err(error)).await;
                    return;
                }

                handle_detection(ctx, trace_id, headers, detectors, input_stream, response_tx)
                    .await;
            }
            .in_current_span(),
        );

        Ok(ReceiverStream::new(response_rx))
    }
}

/// Extracts detectors config from first message.
async fn extract_detectors(
    input_stream: &mut Peekable<InputStream>,
) -> Result<HashMap<String, DetectorParams>, Error> {
    // We can use Peekable to get a reference to it instead of consuming the message here
    // Peekable::peek() takes self: Pin<&mut Peekable<_>>, which is why we need to pin it
    // https://docs.rs/futures/latest/futures/stream/struct.Peekable.html
    if let Some((_index, result)) = Pin::new(input_stream).peek().await {
        match result {
            Ok(msg) => {
                if let Some(detectors) = &msg.detectors {
                    if detectors.is_empty() {
                        return Err(Error::Validation(
                            "`detectors` must not be empty".to_string(),
                        ));
                    }
                    return Ok(detectors.clone());
                }
            }
            Err(error) => return Err(error.clone()),
        }
    }
    Err(Error::Validation(
        "`detectors` is required for the first message".into(),
    ))
}

#[instrument(skip_all)]
async fn handle_detection(
    ctx: Arc<Context>,
    trace_id: TraceId,
    headers: HeaderMap,
    detectors: HashMap<String, DetectorParams>,
    mut input_stream: InputStream,
    response_tx: mpsc::Sender<Result<StreamingContentDetectionResponse, Error>>,
) {
    // Create input channel for detection pipeline
    let (input_tx, input_rx) = mpsc::channel(128);
    // Create detection streams
    let detection_streams =
        common::text_contents_detection_streams(ctx, headers, detectors.clone(), 0, input_rx).await;

    // Spawn task to process detection streams
    tokio::spawn(
        async move {
            match detection_streams {
                Ok(detection_streams) => {
                    // Create detection batch stream
                    let detection_batch_stream = DetectionBatchStream::new(
                        MaxProcessedIndexBatcher::new(detectors.len()),
                        detection_streams,
                    );
                    process_detection_batch_stream(trace_id, detection_batch_stream, response_tx)
                        .await;
                }
                Err(error) => {
                    error!(%trace_id, %error, "task failed: error creating detection streams");
                    // Send error to response channel and terminate
                    let _ = response_tx.send(Err(error)).await;
                }
            }
        }
        .in_current_span(),
    );

    // Spawn task to consume input stream
    tokio::spawn(
        async move {
            while let Some((index, result)) = input_stream.next().await {
                match result {
                    Ok(message) => {
                        // Send content text to input channel
                        let _ = input_tx.send(Ok((index, message.content))).await;
                    }
                    Err(error) => {
                        // Send error to input channel
                        let _ = input_tx.send(Err(error)).await;
                    }
                }
            }
        }
        .in_current_span(),
    );
}

/// Consumes a detection batch stream, builds responses, and sends them to a response channel.
#[instrument(skip_all)]
async fn process_detection_batch_stream(
    trace_id: TraceId,
    mut detection_batch_stream: DetectionBatchStream,
    response_tx: mpsc::Sender<Result<StreamingContentDetectionResponse, Error>>,
) {
    while let Some(result) = detection_batch_stream.next().await {
        match result {
            Ok((_, chunk, detections)) => {
                let response = StreamingContentDetectionResponse {
                    start_index: chunk.start as u32,
                    processed_index: chunk.end as u32,
                    detections: detections.into_iter().map(Into::into).collect(),
                };
                // Send message to response channel
                if response_tx.send(Ok(response)).await.is_err() {
                    info!(%trace_id, "task completed: client disconnected");
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
    info!(%trace_id, "task completed: detection batch stream closed");
}

pub struct StreamingContentDetectionTask {
    /// Trace ID
    pub trace_id: TraceId,
    /// Headers
    pub headers: HeaderMap,
    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,
    /// Input stream to run detections on
    pub input_stream: BoxStream<(usize, Result<StreamingContentDetectionRequest, Error>)>,
}

impl StreamingContentDetectionTask {
    pub fn new(
        trace_id: TraceId,
        headers: HeaderMap,
        input_stream: BoxStream<(usize, Result<StreamingContentDetectionRequest, Error>)>,
    ) -> Self {
        Self {
            trace_id,
            headers,
            detectors: HashMap::default(),
            input_stream,
        }
    }
}
