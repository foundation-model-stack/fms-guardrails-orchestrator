use futures::{stream, Stream, StreamExt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{BoxStream, DetectionStream, Detections};
use crate::orchestrator::Error;

pub struct DetectionBatchStream {
    inner: BoxStream<Result<DetectionBatch, Error>>,
}

impl DetectionBatchStream {
    pub fn new(streams: Vec<DetectionStream>) -> Self {
        let _n = streams.len();
        let (batch_tx, batch_rx) = mpsc::channel(32);
        let (batcher_tx, mut batcher_rx) = mpsc::channel(32);

        // Spawn batcher task
        tokio::spawn(async move {
            // let mut tracker = DetectionTracker::new(n);
            loop {
                tokio::select! {
                    result = batcher_rx.recv() => {
                        match result {
                            Some(Ok(_detections)) => {
                                // tracker.push(detections);
                                todo!()
                            },
                            Some(Err(error)) => {
                                let _ = batch_tx.send(Err(error)).await;
                                break;
                            },
                            None => break,
                        }
                    },
                    // batch = todo!() => {
                    //     let _ = batch_tx.send(Ok(batch)).await;
                    // }
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

impl Stream for DetectionBatchStream {
    type Item = Result<DetectionBatch, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
// pub type Span = (usize, usize);

#[derive(Debug, Clone)]
pub struct DetectionBatch {
    pub span: Span,
    pub detections: Detections,
}
// pub type DetectionBatch = (Span, Detections);

struct DetectionTracker {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detection_tracker() -> Result<(), Error> {
        Ok(())
    }

    #[tokio::test]
    async fn test_detection_batch_stream() -> Result<(), Error> {
        Ok(())
    }
}
