use futures::{stream, Stream, StreamExt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{DetectionStream, Detections};
use crate::orchestrator::Error;

struct DetectionTracker {}

pub struct BatchDetectionStream<T> {
    inner: ReceiverStream<Result<(T, Detections), Error>>, // TODO: update
}

impl<T: Send + 'static> BatchDetectionStream<T> {
    pub fn new(streams: Vec<DetectionStream<T>>) -> Self {
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
            inner: ReceiverStream::new(batch_rx),
        }
    }
}

impl<T> Stream for BatchDetectionStream<T> {
    type Item = Result<(T, Detections), Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}
