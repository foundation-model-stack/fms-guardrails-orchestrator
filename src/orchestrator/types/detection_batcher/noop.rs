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
