use super::{Chunk, DetectionBatcher, Detections, DetectorId, InputId};
use crate::orchestrator::types::Chunks;

/// A batcher for chat completions.
pub struct ChatCompletionBatcher {
    detectors: Vec<DetectorId>,
    // state: TBD
}

impl ChatCompletionBatcher {
    pub fn new(detectors: Vec<DetectorId>) -> Self {
        // let state = TBD::new();
        Self {
            detectors,
            // state,
        }
    }
}

impl DetectionBatcher for ChatCompletionBatcher {
    type Batch = ChatCompletionDetectionBatch; // placeholder, actual type TBD

    fn push(
        &mut self,
        input_id: InputId,
        detector_id: DetectorId,
        chunk: Chunk,
        detections: Detections,
    ) {
        // NOTE: input_id maps to choice_index
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
