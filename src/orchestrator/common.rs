use std::{collections::HashMap, sync::Arc};

use either::Either;
use futures::{stream, StreamExt, TryStreamExt};
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
    inputs: Either<T, Vec<(usize, T)>>,
) -> Result<HashMap<ChunkerId, Chunks>, Error>
where
    T: ToString + Clone + Send + 'static,
{
    let inputs = chunkers.iter().flat_map(|chunker_id| match inputs {
        Either::Left(ref input) => {
            vec![(chunker_id.clone(), 0, input.to_string())]
        }
        Either::Right(ref inputs) => inputs
            .iter()
            .map(|(index, input)| (chunker_id.clone(), *index, input.to_string()))
            .collect::<Vec<_>>(),
    });

    let results = stream::iter(inputs)
        .map(|(chunker_id, index, text)| {
            let ctx = ctx.clone();
            async move {
                let chunks = chunk(ctx, chunker_id.clone(), index, text).await?;
                Ok::<_, Error>((chunker_id, chunks))
            }
        })
        .buffer_unordered(8)
        .try_collect::<HashMap<_, _>>()
        .await?;

    Ok(results)
}

/// Spawns chunk streaming tasks.
/// Returns a map of chunk streams.
pub async fn chunk_streams<T>(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    input_broadcast_tx: broadcast::Sender<(usize, T)>,
) -> Result<HashMap<ChunkerId, ChunkStream>, Error>
where
    T: ToString + Clone + Send + 'static,
{
    todo!()
}

/// Spawns text contents detection tasks.
/// Returns a vec of detections.
pub async fn text_contents_detections(
    ctx: Arc<Context>,
    detectors: Vec<(DetectorId, DetectorParams)>,
    chunks: HashMap<ChunkerId, Chunks>,
) -> Result<Vec<(DetectorId, Detections)>, Error> {
    let inputs = detectors
        .iter()
        .map(|(detector_id, params)| {
            let config = ctx
                .config
                .detectors
                .get(detector_id)
                .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?;
            let chunks = chunks.get(&config.chunker_id).unwrap().clone();
            Ok::<_, Error>((detector_id.clone(), params.clone(), chunks))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    let results = stream::iter(inputs)
        .map(|(detector_id, params, chunks)| {
            let ctx = ctx.clone();
            async move {
                let detections =
                    detect_text_contents(ctx, detector_id.clone(), params, chunks).await?;
                Ok::<_, Error>((detector_id, detections))
            }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await?;

    Ok(results)
}

/// Spawns text contents detection stream tasks.
/// Returns a vec of detection streams.
pub async fn text_contents_detection_streams(
    ctx: Arc<Context>,
    detectors: Vec<(DetectorId, DetectorParams)>,
    chunk_streams: HashMap<ChunkerId, ChunkStream>,
) -> Result<Vec<(DetectorId, DetectionStream)>, Error> {
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
    index: usize,
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

    #[tokio::test]
    async fn test_chunk_streams() -> Result<(), Error> {
        Ok(())
    }

    #[tokio::test]
    async fn test_text_contents_detections() -> Result<(), Error> {
        todo!()
    }
}
