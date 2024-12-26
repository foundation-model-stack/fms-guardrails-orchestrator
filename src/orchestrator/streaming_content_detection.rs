use std::collections::HashMap;
use std::pin::Pin;

use futures::stream::Peekable;
use futures::Stream;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::error;
use tracing::instrument;

use super::{Error, Orchestrator, StreamingContentDetectionTask};
use crate::models::StreamingContentDetectionRequest;
use crate::models::StreamingContentDetectionResponse;

type ContentInputStream =
    Peekable<Pin<Box<dyn Stream<Item = Result<StreamingContentDetectionRequest, Error>> + Send>>>;

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

        let mut input_stream = task.input_stream.peekable();
        let mut processed_index = 0;

        // Create response channel
        #[allow(clippy::type_complexity)]
        let (response_tx, response_rx): (
            mpsc::Sender<Result<StreamingContentDetectionResponse, Error>>,
            mpsc::Receiver<Result<StreamingContentDetectionResponse, Error>>,
        ) = mpsc::channel(32);

        // Spawn task to process input stream
        tokio::spawn(async move {
            let mut _detectors: HashMap<String, crate::models::DetectorParams> =
                match extract_detectors(&mut input_stream).await {
                    Ok(detectors) => detectors,
                    Err(error) => {
                        error!("{:#?}", error);
                        let _ = response_tx.send(Err(error)).await;
                        return;
                    }
                };

            // TODO: figure out a way not to need this bool
            let mut first_frame = true;
            // Process the input stream
            while let Some(result) = input_stream.next().await {
                match result {
                    Ok(msg) => {
                        // Validation for second input stream frame onward
                        if !first_frame {
                            if let Err(error) = msg.validate_subsequent_requests() {
                                error!("{:#?}", error);
                                let _ = response_tx
                                    .send(Err(Error::Validation(error.to_string())))
                                    .await;
                                return;
                            }
                        } else {
                            first_frame = false;
                        }

                        // TODO: actual processing
                        // Send a dummy response for now
                        let _ = response_tx
                            .send(Ok(StreamingContentDetectionResponse {
                                detections: Vec::new(),
                                processed_index: processed_index as u32,
                                ..Default::default()
                            }))
                            .await;
                        processed_index += 1;
                    }
                    Err(error) => {
                        // json deserialization error, send error message and terminate task
                        error!("{:#?}", error);
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

/// Validates first request frame and returns detectors configuration.
async fn extract_detectors(
    input_stream: &mut ContentInputStream,
) -> Result<HashMap<String, crate::models::DetectorParams>, Error> {
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
