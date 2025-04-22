use std::time::Duration;

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
use futures::{Stream, StreamExt, stream};
use tokio::sync::{mpsc, oneshot};

use super::{Chunk, DetectionBatcher, DetectionStream, Detections, DetectorId, InputId};
use crate::orchestrator::Error;

/// A stream adapter that wraps multiple detection streams and
/// produces a stream of batches using a [`DetectionBatcher`]
/// implementation.
///
/// The detection batcher enables flexible batching
/// logic and returned batch types for different use cases.
pub struct DetectionBatchStream<B: DetectionBatcher> {
    batch_rx: mpsc::Receiver<Result<B::Batch, Error>>,
}

impl<B> DetectionBatchStream<B>
where
    B: DetectionBatcher,
{
    pub fn new(batcher: B, streams: Vec<DetectionStream>) -> Self {
        // Create batch channel
        let (batch_tx, batch_rx) = mpsc::channel(32);
        // Create a stream set (single stream) from multiple detection streams
        let mut stream_set = stream::select_all(streams);
        // Create batcher manager
        // This is an actor that manages batcher state rather than using locks.
        let batcher_manager = DetectionBatcherManagerHandle::new(batcher);

        // Spawn task to send batches as they become available
        tokio::spawn({
            let batch_tx = batch_tx.clone();
            let batcher_manager = batcher_manager.clone();
            async move {
                loop {
                    // Pop next batch, if ready
                    if let Some(batch) = batcher_manager.pop().await {
                        // Send batch to batch channel
                        let _ = batch_tx.send(Ok(batch)).await;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        });
        // Spawn task to consume detections and send them to the batcher
        tokio::spawn(async move {
            while let Some(result) = stream_set.next().await {
                match result {
                    Ok((input_id, detector_id, chunk, detections)) => {
                        // Push detections
                        batcher_manager
                            .push(input_id, detector_id, chunk, detections)
                            .await;
                    }
                    Err(error) => {
                        // Send error to batch channel
                        let _ = batch_tx.send(Err(error)).await;
                        break;
                    }
                }
            }
        });

        Self { batch_rx }
    }
}

impl<B> Stream for DetectionBatchStream<B>
where
    B: DetectionBatcher,
{
    type Item = Result<B::Batch, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.batch_rx.poll_recv(cx)
    }
}

enum DetectionBatcherMessage<Batch> {
    Push {
        input_id: InputId,
        detector_id: DetectorId,
        chunk: Chunk,
        detections: Detections,
    },
    Pop {
        response_tx: oneshot::Sender<Option<Batch>>,
    },
}
/// An actor that manages a [`DetectionBatcher`].
struct DetectionBatcherManager<B: DetectionBatcher> {
    batcher: B,
    rx: mpsc::Receiver<DetectionBatcherMessage<B::Batch>>,
}

impl<B> DetectionBatcherManager<B>
where
    B: DetectionBatcher,
{
    pub fn new(batcher: B, rx: mpsc::Receiver<DetectionBatcherMessage<B::Batch>>) -> Self {
        Self { batcher, rx }
    }

    async fn run(&mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                DetectionBatcherMessage::Push {
                    input_id,
                    detector_id,
                    chunk,
                    detections,
                } => self.batcher.push(input_id, detector_id, chunk, detections),
                DetectionBatcherMessage::Pop { response_tx } => {
                    let batch = self.batcher.pop_batch();
                    let _ = response_tx.send(batch);
                }
            }
        }
    }
}

/// A handle to a [`DetectionBatcherManager`].
#[derive(Clone)]
struct DetectionBatcherManagerHandle<B: DetectionBatcher> {
    tx: mpsc::Sender<DetectionBatcherMessage<B::Batch>>,
}

impl<B> DetectionBatcherManagerHandle<B>
where
    B: DetectionBatcher,
    B::Batch: Clone,
{
    /// Creates a new [`DetectionBatcherManager`] and returns it's handle.
    pub fn new(batcher: B) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let mut actor = DetectionBatcherManager::new(batcher, rx);
        tokio::spawn(async move { actor.run().await });
        Self { tx }
    }

    /// Pushes new detections to the batcher.
    pub async fn push(
        &self,
        input_id: InputId,
        detector_id: DetectorId,
        chunk: Chunk,
        detections: Detections,
    ) {
        let _ = self
            .tx
            .send(DetectionBatcherMessage::Push {
                input_id,
                detector_id,
                chunk,
                detections,
            })
            .await;
    }

    /// Removes the next batch of detections from the batcher, if ready.
    pub async fn pop(&self) -> Option<B::Batch> {
        let (response_tx, response_rx) = oneshot::channel();
        let _ = self
            .tx
            .send(DetectionBatcherMessage::Pop { response_tx })
            .await;
        response_rx.await.unwrap_or_default()
    }
}
