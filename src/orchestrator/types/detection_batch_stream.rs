use futures::{Stream, StreamExt, stream};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{BoxStream, Chunk, DetectionBatcher, DetectionStream, Detections, DetectorId, InputId};
use crate::orchestrator::Error;

/// Wraps detection streams and produces a stream of batches using a [`DetectionBatcher`].
pub struct DetectionBatchStream<B: DetectionBatcher> {
    inner: BoxStream<Result<B::Batch, Error>>,
}

impl<B> DetectionBatchStream<B>
where
    B: DetectionBatcher,
{
    pub fn new(mut batcher: B, streams: Vec<DetectionStream>) -> Self {
        let (batch_tx, batch_rx) = mpsc::channel(32);
        let (batcher_tx, mut batcher_rx) =
            mpsc::channel::<Result<(InputId, DetectorId, Chunk, Detections), Error>>(32);

        // Spawn batcher task
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = batcher_rx.recv() => {
                        match result {
                            Some(Ok((input_id, detector_id, chunk, detections))) => {
                                batcher.push(input_id, detector_id, chunk, detections);
                                if let Some(batch) = batcher.pop_batch() {
                                    let _ = batch_tx.send(Ok(batch)).await;
                                }
                            },
                            Some(Err(error)) => {
                                let _ = batch_tx.send(Err(error)).await;
                                break;
                            },
                            None => break,
                        }
                    },
                }
            }
        });

        // Spawn detection consumer task
        let mut stream_set = stream::select_all(streams);
        tokio::spawn(async move {
            while let Some(result) = stream_set.next().await {
                // Send to batcher task
                let _ = batcher_tx.send(result).await;
            }
        });

        Self {
            inner: ReceiverStream::new(batch_rx).boxed(),
        }
    }
}

impl<T: DetectionBatcher> Stream for DetectionBatchStream<T> {
    type Item = Result<T::Batch, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}
