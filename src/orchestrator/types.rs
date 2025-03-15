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
use std::pin::Pin;

use futures::Stream;

pub mod chat_message;
pub use chat_message::*;
pub mod chunk;
pub mod detection;
pub use chunk::*;
pub use detection::*;
pub mod detection_batcher;
pub use detection_batcher::*;
pub mod detection_batch_stream;
pub use detection_batch_stream::*;

use super::Error;
use crate::{clients::openai::ChatCompletionChunk, models::ClassifiedGeneratedTextStreamResult};

pub type ChunkerId = String;
pub type DetectorId = String;
pub type InputId = u32;

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;
pub type ChunkStream = BoxStream<Result<Chunk, Error>>;
pub type InputStream = BoxStream<Result<(usize, String), Error>>;
pub type DetectionStream = BoxStream<Result<(InputId, DetectorId, Chunk, Detections), Error>>;
pub type GenerationStream = BoxStream<(usize, Result<ClassifiedGeneratedTextStreamResult, Error>)>;
pub type ChatCompletionStream = BoxStream<(usize, Result<Option<ChatCompletionChunk>, Error>)>;
