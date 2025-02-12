use std::{collections::HashMap, sync::Arc};

use either::Either;
use tokio::sync::broadcast;

use super::{types::*, Context, Error};
use crate::{
    clients::{detector::ContextType, openai},
    models::{
        ClassifiedGeneratedTextResult as GenerateResponse,
        ClassifiedGeneratedTextStreamResult as GenerateStreamResponse, DetectorParams,
        GuardrailsTextGenerationParameters as GenerateParams,
    },
};

pub mod utils;

// Processing tasks

/// Spawns chunk tasks.
/// Returns a map of chunks.
pub async fn chunks<T>(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    inputs: Either<T, impl IntoIterator<Item = (usize, T)>>,
) -> Result<HashMap<ChunkerId, Vec<Chunked<T>>>, Error>
where
    T: ToString + Clone + Send + 'static,
{
    // Collect to (offset, text, input) tuple
    let inputs = match inputs {
        Either::Left(input) => vec![(0, input.to_string(), input)],
        Either::Right(inputs) => inputs
            .into_iter()
            .map(|(offset, input)| (offset, input.to_string(), input))
            .collect::<Vec<_>>(),
    };
    todo!()
}

/// Spawns chunk streaming tasks.
/// Returns a map of chunk streams.
pub async fn chunk_streams<T>(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    input_broadcast_tx: broadcast::Sender<(usize, T)>,
) -> Result<HashMap<ChunkerId, ChunkStream<T>>, Error>
where
    T: ToString + Clone + Send + 'static,
{
    todo!()
}

/// Spawns text contents detection tasks.
/// Returns a vec of detections.
pub async fn text_contents_detections<T>(
    ctx: Arc<Context>,
    detectors: Vec<(DetectorId, DetectorParams)>,
    chunks: HashMap<ChunkerId, Vec<Chunked<T>>>,
) -> Result<Vec<(DetectorId, Detections)>, Error>
where
    T: ToString + Clone + Send + 'static,
{
    todo!()
}

/// Spawns text contents detection stream tasks.
/// Returns a vec of detection streams.
pub async fn text_contents_detection_streams<T>(
    ctx: Arc<Context>,
    detectors: Vec<(DetectorId, DetectorParams)>,
    chunk_streams: HashMap<ChunkerId, ChunkStream<T>>,
) -> Result<Vec<(DetectorId, DetectionStream<T>)>, Error>
where
    T: ToString + Clone + Send + 'static,
{
    todo!()
}

/// Spawns text generation detection tasks.
/// Returns a vec of detections.
pub async fn text_generation_detections(
    ctx: Arc<Context>,
    detectors: Vec<(DetectorId, DetectorParams)>,
    prompt: String,
    generated_text: String,
) -> Result<Vec<(DetectorId, Detections)>, Error> {
    todo!()
}

/// Spawns text chat detection tasks.
/// Returns a vec of detections.
pub async fn text_chat_detections(
    ctx: Arc<Context>,
    detectors: Vec<(String, DetectorParams)>,
    messages: Vec<openai::Message>,
) -> Result<Vec<(String, Detections)>, Error> {
    todo!()
}

/// Spawns text context detection tasks.
/// Returns a vec of detections.
pub async fn text_context_detections(
    ctx: Arc<Context>,
    detectors: Vec<(String, DetectorParams)>,
    text: String,
    context_type: ContextType,
    context: Vec<String>,
) -> Result<Vec<(String, Detections)>, Error> {
    todo!()
}

// Client call helpers

/// Sends request to chunker client.
pub async fn chunk(
    ctx: Arc<Context>,
    chunker_id: String,
    offset: usize,
    text: String,
) -> Result<Chunks, Error> {
    todo!()
}

/// Sends request to text contents detector client.
pub async fn detect_text_contents(
    ctx: Arc<Context>,
    detector_id: String,
    params: DetectorParams,
    chunks: Vec<Chunk>,
) -> Result<(String, Vec<Detection>), Error> {
    todo!()
}

/// Sends request to text generation detector client.
pub async fn detect_text_generation(
    ctx: Arc<Context>,
    detector_id: String,
    params: DetectorParams,
    prompt: String,
    generated_text: String,
) -> Result<(String, Vec<Detection>), Error> {
    todo!()
}

/// Sends request to text chat detector client.
pub async fn detect_text_chat(
    ctx: Arc<Context>,
    detector_id: String,
    params: DetectorParams,
    messages: Vec<openai::Message>,
) -> Result<(String, Vec<Detection>), Error> {
    todo!()
}

/// Sends request to text context detector client.
pub async fn detect_text_context(
    ctx: Arc<Context>,
    detector_id: String,
    params: DetectorParams,
    context_type: String,
    context: String,
) -> Result<(String, Vec<Detection>), Error> {
    todo!()
}

/// Sends request to openai chat completions client.
pub async fn chat_completions(
    ctx: Arc<Context>,
    request: openai::ChatCompletionsRequest,
) -> Result<openai::ChatCompletionsResponse, Error> {
    todo!()
}

/// Sends tokenize request to generation client.
pub async fn tokenize(
    ctx: Arc<Context>,
    model_id: String,
    text: String,
) -> Result<(u32, Vec<String>), Error> {
    todo!()
}

/// Sends generate request to generation client.
pub async fn generate(
    ctx: Arc<Context>,
    model_id: String,
    text: String,
    params: Option<GenerateParams>,
) -> Result<GenerateResponse, Error> {
    todo!()
}

/// Sends generate stream request to generation client.
pub async fn generate_stream(
    ctx: Arc<Context>,
    model_id: String,
    text: String,
    params: Option<GenerateParams>,
) -> Result<BoxStream<Result<GenerateStreamResponse, Error>>, Error> {
    todo!()
}
