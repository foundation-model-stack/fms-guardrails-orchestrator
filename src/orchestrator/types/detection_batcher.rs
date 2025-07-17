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
pub mod completion;
pub use completion::*;
pub mod max_processed_index;
pub use max_processed_index::*;

use super::{Chunk, Detections};

pub type Batch = (u32, Chunk, Detections);

/// A detection batcher.
/// Implements pluggable batching logic for a [`DetectionBatchStream`].
pub trait DetectionBatcher: std::fmt::Debug + Clone + Send + 'static {
    /// Pushes new detections.
    fn push(&mut self, input_id: u32, chunk: Chunk, detections: Detections);

    /// Removes the next batch of detections, if ready.
    fn pop_batch(&mut self) -> Option<Batch>;

    /// Returns `true` if the batcher state is empty.
    fn is_empty(&self) -> bool;
}
