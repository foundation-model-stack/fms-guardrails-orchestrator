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
