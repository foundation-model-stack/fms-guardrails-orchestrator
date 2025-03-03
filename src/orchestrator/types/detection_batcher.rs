pub mod chat_completion;
pub use chat_completion::*;
pub mod noop;
pub use noop::*;
pub mod max_processed_index;
pub use max_processed_index::*;

use super::{Chunk, Detections, DetectorId, InputId};

/// A detection batcher.
/// Pluggable batching logic for a [`DetectionBatchStream`].
pub trait DetectionBatcher: Send + 'static {
    type Batch: Send + 'static;

    fn push(
        &mut self,
        input_id: InputId,
        detector_id: DetectorId,
        chunk: Chunk,
        detections: Detections,
    );

    fn pop_batch(&mut self) -> Option<Self::Batch>;
}
