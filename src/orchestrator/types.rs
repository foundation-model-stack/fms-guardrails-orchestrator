use std::pin::Pin;

use futures::Stream;

pub mod chat_message;
pub use chat_message::{ChatMessage, ChatMessageIterator};
pub mod chunk;
pub mod detection;
pub use chunk::{Chunk, Chunks};
pub use detection::{Detection, Detections};
pub mod detection_batch_stream;
pub use detection_batch_stream::{
    ChatCompletionBatcher, CompletedChunkBatcher, DetectionBatchStream, FakeBatcher,
};

use super::Error;
use crate::models;

pub type ChunkerId = String;
pub type DetectorId = String;

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;
pub type ChunkStream = BoxStream<Result<Chunk, Error>>;
pub type DetectionStream = BoxStream<Result<(DetectorId, Chunk, Detections), Error>>;
pub type GenerationStream =
    BoxStream<Result<(usize, models::ClassifiedGeneratedTextStreamResult), Error>>;

#[allow(clippy::to_string_trait_impl)]
impl ToString for models::ClassifiedGeneratedTextStreamResult {
    fn to_string(&self) -> String {
        self.generated_text.clone().unwrap_or_default()
    }
}
