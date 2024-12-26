use std::{collections::HashMap, pin::Pin};

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::instrument;

use super::{Error, Orchestrator, StreamingContentDetectionTask};
use crate::models::StreamingContentDetectionResponse;

impl Orchestrator {
    /// Handles content detection streaming tasks.
    #[instrument(skip_all, fields(trace_id = task.trace_id.to_string(), headers = ?task.headers))]
    pub async fn handle_streaming_content_detection(
        &self,
        task: StreamingContentDetectionTask,
    ) -> ReceiverStream<Result<StreamingContentDetectionResponse, Error>> {
        let _ctx = self.ctx.clone();
        let _trace_id = task.trace_id;
        let _headers = task.headers;

        let mut input_stream = task.input_stream;
        let mut processed_index = 0;

        // Create response channel
        #[allow(clippy::type_complexity)]
        let (response_tx, response_rx): (
            mpsc::Sender<Result<StreamingContentDetectionResponse, Error>>,
            mpsc::Receiver<Result<StreamingContentDetectionResponse, Error>>,
        ) = mpsc::channel(32);

        // Spawn task to process input stream
        tokio::spawn(async move {
            let mut detectors: HashMap<String, crate::models::DetectorParams>;
            // Get detector config from the first message
            // We can use Peekable to get a reference to it instead of consuming the message here
            // Peekable::peek() takes self: Pin<&mut Peekable<_>>, which is why we need to pin it
            // https://docs.rs/futures/latest/futures/stream/struct.Peekable.html
            if let Some(result) = input_stream.next().await {
                match result {
                    Ok(msg) => {
                        // validate initial stream frame
                        if let Err(error) = msg.validate_initial_request() {
                            tracing::error!("{:#?}", error);
                            let _ = response_tx
                                .send(Err(Error::Validation(error.to_string())))
                                .await;
                            return;
                        }

                        if let Some(d) = &msg.detectors {
                            detectors = d.clone();
                        } else {
                            // No detectors configured, send error message and terminate task
                            let _ = response_tx
                                .send(Err(Error::Validation("no detectors configured".into())))
                                .await;
                            return;
                        }

                        let _ = response_tx
                            .send(Ok(StreamingContentDetectionResponse {
                                detections: Vec::new(),
                                processed_index,
                                ..Default::default()
                            }))
                            .await;
                        processed_index += 1;
                    }
                    Err(error) => {
                        // json deserialization error, send error message and terminate task
                        tracing::error!("{:#?}", error);
                        let _ = response_tx
                            .send(Err(Error::Validation(error.to_string())))
                            .await;
                        return;
                    }
                }
            }
            // Process the input stream
            while let Some(result) = input_stream.next().await {
                match result {
                    Ok(msg) => {
                        if let Err(error) = msg.validate_subsequent_requests() {
                            tracing::error!("{:#?}", error);
                            let _ = response_tx
                                .send(Err(Error::Validation(error.to_string())))
                                .await;
                            return;
                        }

                        // TODO: actual processing
                        // Send a dummy response for now
                        let _ = response_tx
                            .send(Ok(StreamingContentDetectionResponse {
                                detections: Vec::new(),
                                processed_index,
                                ..Default::default()
                            }))
                            .await;
                        processed_index += 1;
                    }
                    Err(error) => {
                        // json deserialization error, send error message and terminate task
                        let _ = response_tx
                            .send(Err(Error::Validation(error.to_string())))
                            .await;
                        return;
                    }
                }
            }
        });
        ReceiverStream::new(response_rx)
    }
}
