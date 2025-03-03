use std::pin::Pin;

use futures::Stream;
use tokio::sync::mpsc;

pub mod chat_message;
pub use chat_message::{ChatMessage, ChatMessageIterator};
pub mod chunk;
pub mod detection;
pub use chunk::{Chunk, Chunks};
pub use detection::{Detection, Detections};
pub mod detection_batcher;
pub use detection_batcher::*;
pub mod detection_batch_stream;
pub use detection_batch_stream::DetectionBatchStream;

use super::Error;
use crate::{clients::openai::ChatCompletionChunk, models::ClassifiedGeneratedTextStreamResult};

pub type ChunkerId = String;
pub type DetectorId = String;
pub type InputId = u32;

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;
pub type ChunkStream = BoxStream<Result<Chunk, Error>>;
pub type InputStream = BoxStream<Result<(usize, String), Error>>;
pub type InputSender = mpsc::Sender<Result<(usize, String), Error>>;
pub type InputReceiver = mpsc::Receiver<Result<(usize, String), Error>>;
pub type DetectionStream = BoxStream<Result<(InputId, DetectorId, Chunk, Detections), Error>>;
pub type GenerationStream = BoxStream<(usize, Result<ClassifiedGeneratedTextStreamResult, Error>)>;
pub type ChatCompletionStream = BoxStream<(usize, Result<Option<ChatCompletionChunk>, Error>)>;
