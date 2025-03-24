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
use tokio::sync::mpsc;

use super::{DetectionBatcher, DetectionStream};
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
    pub fn new(mut batcher: B, streams: Vec<DetectionStream>) -> Self {
        // Create batch channel
        let (batch_tx, batch_rx) = mpsc::channel(32);

        // Create a stream set (single stream) from multiple detection streams
        let mut stream_set = stream::select_all(streams);

        // Spawn batcher task
        // This task consumes new detections, pushes them to the batcher,
        // and sends batches to the batch channel as they become ready.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = stream_set.next() => {
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
                                // Detection stream set closed
                                break;
                            },
                        }
                    },
                }
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
