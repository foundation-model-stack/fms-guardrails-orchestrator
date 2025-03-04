#![allow(dead_code)]
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
    type Batch = (u32, Chunks, Detections); // placeholder, actual type TBD

    fn push(
        &mut self,
        _input_id: InputId,
        _detector_id: DetectorId,
        _chunk: Chunk,
        _detections: Detections,
    ) {
        // NOTE: input_id maps to choice_index
        todo!()
    }

    fn pop_batch(&mut self) -> Option<Self::Batch> {
        // TODO: implement batching logic to align with requirements
        // ref: https://github.com/foundation-model-stack/fms-guardrails-orchestrator/blob/main/docs/architecture/adrs/005-chat-completion-support.md#streaming-response
        todo!()
    }
}
