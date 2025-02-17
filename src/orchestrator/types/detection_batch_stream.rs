use std::collections::{btree_map, BTreeMap, VecDeque};

use futures::{stream, Stream, StreamExt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{BoxStream, Chunk, Chunks, DetectionStream, Detections};
use crate::orchestrator::{types::DetectorId, Error};

pub struct DetectionBatchStream<B: DetectionBatcher> {
    inner: BoxStream<Result<B::Batch, Error>>,
}

impl<B> DetectionBatchStream<B>
where
    B: DetectionBatcher,
{
    pub fn new(mut batcher: B, streams: Vec<(DetectorId, DetectionStream)>) -> Self {
        let (detectors, streams): (Vec<_>, Vec<_>) = streams.into_iter().unzip();
        let (batch_tx, batch_rx) = mpsc::channel(32);
        let (batcher_tx, mut batcher_rx) =
            mpsc::channel::<Result<(DetectorId, Chunk, Detections), Error>>(32);

        // Spawn batcher task
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = batcher_rx.recv() => {
                        match result {
                            Some(Ok((detector_id, chunk, detections))) => {
                                batcher.push(detector_id, chunk, detections);
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

pub trait DetectionBatcher: Send + 'static {
    type Batch: Send + 'static;

    fn push(&mut self, detector_id: DetectorId, chunk: Chunk, detections: Detections);
    fn pop_batch(&mut self) -> Option<Self::Batch>;
}

/// A batcher implementation based on the original tracker.
/// Does not support detections with different chunkers applied.
struct SimpleBatcher {
    detectors: Vec<DetectorId>,
    state: BTreeMap<Span, (Chunk, Vec<Detections>)>,
}

impl SimpleBatcher {
    pub fn new(detectors: Vec<DetectorId>) -> Self {
        Self {
            detectors,
            state: BTreeMap::default(),
        }
    }
}

impl DetectionBatcher for SimpleBatcher {
    type Batch = DetectionBatch;

    fn push(&mut self, _detector_id: DetectorId, chunk: Chunk, detections: Detections) {
        let span = Span::new(chunk.start, chunk.end);
        match self.state.entry(span) {
            btree_map::Entry::Vacant(entry) => {
                // New span, insert entry with chunk and detections
                entry.insert((chunk, vec![detections]));
            }
            btree_map::Entry::Occupied(mut entry) => {
                // Existing span, push detections
                entry.get_mut().1.push(detections);
            }
        }
    }

    fn pop_batch(&mut self) -> Option<Self::Batch> {
        // Check if we have all detections for the first span
        if self
            .state
            .first_key_value()
            .is_some_and(|(_, (chunk, detections))| detections.len() == self.detectors.len())
        {
            if let Some((span, (chunk, detections))) = self.state.pop_first() {
                let detections = detections.into_iter().flatten().collect();
                Some(DetectionBatch { chunk, detections })
            } else {
                None
            }
        } else {
            None
        }
    }
}

/// A batcher implementation that doesn't actually batch.
struct FakeBatcher {
    detectors: Vec<DetectorId>,
    state: VecDeque<(Chunk, Detections)>,
}

impl FakeBatcher {
    pub fn new(detectors: Vec<DetectorId>) -> Self {
        Self {
            detectors,
            state: VecDeque::default(),
        }
    }
}

impl DetectionBatcher for FakeBatcher {
    type Batch = DetectionBatch;

    fn push(&mut self, _detector_id: DetectorId, chunk: Chunk, detections: Detections) {
        self.state.push_back((chunk, detections));
    }

    fn pop_batch(&mut self) -> Option<Self::Batch> {
        self.state
            .pop_front()
            .map(|(chunk, detections)| DetectionBatch { chunk, detections })
    }
}

#[derive(Debug, Clone)]
pub struct DetectionBatch {
    pub chunk: Chunk,
    pub detections: Detections,
}

/// A batcher implementation for chat completion detections.
struct ChatCompletionBatcher {
    detectors: Vec<DetectorId>,
    //state
}

impl ChatCompletionBatcher {
    pub fn new(detectors: Vec<DetectorId>) -> Self {
        Self {
            detectors,
            //state,
        }
    }
}

impl DetectionBatcher for ChatCompletionBatcher {
    type Batch = ChatCompletionDetectionBatch;

    fn push(&mut self, detector_id: DetectorId, chunk: Chunk, detections: Detections) {
        // let choice_index = chunk.index;
        todo!()
    }

    fn pop_batch(&mut self) -> Option<Self::Batch> {
        todo!()
    }
}

#[derive(Debug, Clone)]
pub struct ChatCompletionDetectionBatch {
    pub choice_index: usize,
    pub chunks: Chunks,
    pub detections: Detections,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detection_batch_stream() -> Result<(), Error> {
        let batcher = SimpleBatcher::new(vec!["d1".to_string(), "d2".to_string()]);
        let d1_stream = stream::iter(vec![
            Ok(("d1".to_string(), Chunk::default(), Detections::default())),
            Ok(("d1".to_string(), Chunk::default(), Detections::default())),
            Ok(("d1".to_string(), Chunk::default(), Detections::default())),
        ])
        .boxed();
        let d2_stream = stream::iter(vec![
            Ok(("d2".to_string(), Chunk::default(), Detections::default())),
            Ok(("d2".to_string(), Chunk::default(), Detections::default())),
            Ok(("d2".to_string(), Chunk::default(), Detections::default())),
        ])
        .boxed();
        let streams = vec![("d1".to_string(), d1_stream), ("d2".to_string(), d2_stream)];
        let batch_stream = DetectionBatchStream::new(batcher, streams);

        Ok(())
    }

    #[test]
    fn test_simple_batcher() -> Result<(), Error> {
        Ok(())
    }

    #[test]
    fn test_chat_completions_batcher() -> Result<(), Error> {
        Ok(())
    }
}
