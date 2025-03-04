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
pub mod chat_completion;
pub use chat_completion::*;
pub mod noop;
pub use noop::*;
pub mod max_processed_index;
pub use max_processed_index::*;

use super::{Chunk, Detections, DetectorId, InputId};

/// A detection batcher.
/// Implements pluggable batching logic for a [`DetectionBatchStream`].
pub trait DetectionBatcher: Send + 'static {
    type Batch: Send + 'static;

    /// Pushes new detections.
    fn push(
        &mut self,
        input_id: InputId,
        detector_id: DetectorId,
        chunk: Chunk,
        detections: Detections,
    );

    /// Removes the next batch of detections, if ready.
    fn pop_batch(&mut self) -> Option<Self::Batch>;
}
