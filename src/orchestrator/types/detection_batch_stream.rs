use futures::{stream, Stream, StreamExt};
use tokio::sync::mpsc;

use super::{Chunk, DetectionBatcher, DetectionStream, Detections, DetectorId, InputId};
use crate::orchestrator::Error;

/// Wraps detection streams and produces a stream
/// of batches using a [`DetectionBatcher`].
pub struct DetectionBatchStream<B: DetectionBatcher> {
    batch_rx: mpsc::Receiver<Result<B::Batch, Error>>,
}

impl<B> DetectionBatchStream<B>
where
    B: DetectionBatcher,
{
    pub fn new(mut batcher: B, streams: Vec<DetectionStream>) -> Self {
        // Create batch channel
        let (batch_tx, batch_rx) = mpsc::channel(32);
        // Create batcher channel
        let (batcher_tx, mut batcher_rx) =
            mpsc::channel::<Result<(InputId, DetectorId, Chunk, Detections), Error>>(32);

        // Spawn batcher task
        // This task receives new detections, pushes them to the batcher,
        // and sends batches to the batch (output) channel as they become ready.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = batcher_rx.recv() => {
                        match result {
                            Some(Ok((input_id, detector_id, chunk, detections))) => {
                                // Push detections to batcher
                                batcher.push(input_id, detector_id, chunk, detections);

                                // Check if the next batch is ready
                                if let Some(batch) = batcher.pop_batch() {
                                    // Send batch to batch channel
                                    let _ = batch_tx.send(Ok(batch)).await;
                                }
                            },
                            Some(Err(error)) => {
                                // Send error to batch channel
                                let _ = batch_tx.send(Err(error)).await;
                                break;
                            },
                            None => {
                                // Batcher channel closed
                                break;
                            },
                        }
                    },
                }
            }
        });

        // Create a stream set (single stream) from multiple detection streams
        let mut stream_set = stream::select_all(streams);

        // Spawn detection consumer task
        // This task consumes new detections and sends them to the batcher task.
        tokio::spawn(async move {
            while let Some(result) = stream_set.next().await {
                // Send new detections to batcher task
                let _ = batcher_tx.send(result).await;
            }
        });

        Self { batch_rx }
    }
}

impl<T: DetectionBatcher> Stream for DetectionBatchStream<T> {
    type Item = Result<T::Batch, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.batch_rx.poll_recv(cx)
    }
}
