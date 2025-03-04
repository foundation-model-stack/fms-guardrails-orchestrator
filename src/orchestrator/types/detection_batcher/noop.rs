use std::collections::VecDeque;

use super::{Chunk, DetectionBatcher, Detections, DetectorId, InputId};

/// A no-op batcher that doesn't actually batch.
#[derive(Default)]
pub struct NoopBatcher {
    state: VecDeque<(Chunk, Detections)>,
}

impl NoopBatcher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DetectionBatcher for NoopBatcher {
    type Batch = (Chunk, Detections);

    fn push(
        &mut self,
        _input_id: InputId,
        _detector_id: DetectorId,
        chunk: Chunk,
        detections: Detections,
    ) {
        self.state.push_back((chunk, detections));
    }

    fn pop_batch(&mut self) -> Option<Self::Batch> {
        self.state.pop_front()
    }
}
