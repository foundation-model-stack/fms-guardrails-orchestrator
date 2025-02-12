use std::{
    collections::{hash_map, HashMap},
    sync::Arc,
};

use either::Either;
use futures::future::BoxFuture;
use pumps::{Concurrency, Pipeline};
use tokio::sync::{broadcast, mpsc};

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
) -> Result<ChunkMap<T>, Error>
where
    T: ToString + Clone + Send + 'static,
{
    // Collect to (input, offset, text) tuples
    let inputs = match inputs {
        Either::Left(input) => {
            let text = input.to_string();
            vec![(input, 0, text)]
        }
        Either::Right(inputs) => inputs
            .into_iter()
            .map(|(offset, input)| {
                let text = input.to_string();
                (input, offset, text)
            })
            .collect::<Vec<_>>(),
    };
    let capacity = chunkers.len() * inputs.len();
    let (mut chunk_rx, _handle) = chunk_pipeline(ctx.clone(), chunkers, inputs).await;

    // Build chunk map
    let mut chunk_map: ChunkMap<T> = HashMap::with_capacity(capacity);
    while let Some(result) = chunk_rx.recv().await {
        let (chunker_id, input, chunks) = result?;
        match chunk_map.entry(chunker_id) {
            hash_map::Entry::Occupied(mut entry) => {
                entry.get_mut().push((input, chunks));
            }
            hash_map::Entry::Vacant(entry) => {
                entry.insert(vec![(input, chunks)]);
            }
        }
    }
    Ok(chunk_map)
}

async fn chunk_pipeline<T>(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    inputs: Vec<(T, usize, String)>, // (input, offset, text)
) -> (
    mpsc::Receiver<Result<(ChunkerId, T, Chunks), Error>>,
    BoxFuture<'static, Result<(), tokio::task::JoinError>>,
)
where
    T: Clone + Send + 'static,
{
    let mut chunk_inputs = Vec::with_capacity(chunkers.len() * inputs.len());
    for chunker_id in &chunkers {
        for (input, offset, text) in &inputs {
            chunk_inputs.push((chunker_id.clone(), input.clone(), *offset, text.clone()));
        }
    }
    Pipeline::from_iter(chunk_inputs)
        .map(
            move |(chunker_id, input, offset, text)| {
                let ctx = ctx.clone();
                async move {
                    let chunks = chunk(ctx, chunker_id.clone(), offset, text).await?;
                    Ok::<_, Error>((chunker_id, input, chunks))
                }
            },
            Concurrency::concurrent_unordered(8),
        )
        .build()
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
) -> Result<Vec<(DetectorId, Detections)>, Error> {
    todo!()
}

/// Spawns text context detection tasks.
/// Returns a vec of detections.
pub async fn text_context_detections(
    ctx: Arc<Context>,
    detectors: Vec<(DetectorId, DetectorParams)>,
    text: String,
    context_type: ContextType,
    context: Vec<String>,
) -> Result<Vec<(DetectorId, Detections)>, Error> {
    todo!()
}

// Client call helpers

/// Sends request to chunker client.
pub async fn chunk(
    ctx: Arc<Context>,
    chunker_id: ChunkerId,
    offset: usize,
    text: String,
) -> Result<Chunks, Error> {
    todo!()
}

/// Sends request to text contents detector client.
pub async fn detect_text_contents(
    ctx: Arc<Context>,
    detector_id: DetectorId,
    params: DetectorParams,
    chunks: Chunks,
) -> Result<Detections, Error> {
    todo!()
}

/// Sends request to text generation detector client.
pub async fn detect_text_generation(
    ctx: Arc<Context>,
    detector_id: DetectorId,
    params: DetectorParams,
    prompt: String,
    generated_text: String,
) -> Result<Detections, Error> {
    todo!()
}

/// Sends request to text chat detector client.
pub async fn detect_text_chat(
    ctx: Arc<Context>,
    detector_id: DetectorId,
    params: DetectorParams,
    messages: Vec<openai::Message>,
) -> Result<Detections, Error> {
    todo!()
}

/// Sends request to text context detector client.
pub async fn detect_text_context(
    ctx: Arc<Context>,
    detector_id: DetectorId,
    params: DetectorParams,
    context_type: String,
    context: String,
) -> Result<Detections, Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_chunks() -> Result<(), Error> {
        Ok(())
    }
}
