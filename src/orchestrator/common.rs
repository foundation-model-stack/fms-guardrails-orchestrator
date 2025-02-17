use std::{collections::HashMap, sync::Arc};

use futures::{stream, StreamExt, TryStreamExt};
use http::HeaderMap;
use tokio::sync::broadcast;
use tracing::{debug, instrument};

use super::{types::*, Context, Error};
use crate::{
    clients::{
        chunker::ChunkerClient,
        detector::{
            ChatDetectionRequest, ContentAnalysisRequest, ContextDocsDetectionRequest, ContextType,
            GenerationDetectionRequest, TextChatDetectorClient, TextContextDocDetectorClient,
            TextGenerationDetectorClient,
        },
        openai::{self, OpenAiClient},
        GenerationClient, TextContentsDetectorClient,
    },
    models::{
        ClassifiedGeneratedTextResult as GenerateResponse,
        ClassifiedGeneratedTextStreamResult as GenerateStreamResponse, DetectorParams,
        GuardrailsTextGenerationParameters as GenerateParams,
    },
    pb::caikit::runtime::chunkers::ChunkerTokenizationTaskRequest,
};

pub mod ext;
pub mod utils;
pub use utils::*;

// Processing tasks

/// Spawns chunk tasks. Returns a map of chunks.
#[instrument(skip_all)]
pub async fn chunks<T>(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    inputs: Vec<(usize, T)>,
) -> Result<HashMap<ChunkerId, Chunks>, Error>
where
    T: ToString + Clone + Send,
{
    let inputs = chunkers
        .iter()
        .flat_map(|chunker_id| {
            inputs
                .iter()
                .map(|(index, input)| (chunker_id.clone(), *index, input.to_string()))
        })
        .collect::<Vec<_>>();

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

/// Spawns chunk streaming tasks. Returns a map of chunk streams.
#[instrument(skip_all)]
pub async fn chunk_streams<T>(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    input_broadcast_tx: broadcast::Sender<(usize, T)>, // (index, input)
) -> Result<HashMap<ChunkerId, ChunkStream>, Error>
where
    T: ToString + Clone + Send,
{
    todo!()
}

/// Spawns text contents detection tasks.
/// Returns a vec of detections.
#[instrument(skip_all)]
pub async fn text_contents_detections<T>(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: &HashMap<DetectorId, DetectorParams>,
    inputs: Vec<(usize, T)>,
) -> Result<Vec<(DetectorId, Detections)>, Error>
where
    T: ToString + Clone + Send,
{
    let chunkers = get_chunker_ids(&ctx.config, detectors)?;
    let chunks = chunks(ctx.clone(), chunkers, inputs).await?;

    let inputs = detectors
        .iter()
        .map(|(detector_id, params)| {
            let config = ctx
                .config
                .detector(detector_id)
                .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?;
            let chunks = chunks.get(&config.chunker_id).unwrap().clone();
            Ok::<_, Error>((detector_id.clone(), params.clone(), chunks))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    let results = stream::iter(inputs)
        .map(|(detector_id, mut params, chunks)| {
            let ctx = ctx.clone();
            let headers = headers.clone();
            let threshold = params.pop_threshold().unwrap_or_default();
            async move {
                let detections =
                    detect_text_contents(ctx, headers, detector_id.clone(), params, chunks)
                        .await?
                        .into_iter()
                        .filter(|detection| detection.score >= threshold)
                        .collect();
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
#[instrument(skip_all)]
pub async fn text_contents_detection_streams(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: &HashMap<DetectorId, DetectorParams>,
    chunk_streams: HashMap<ChunkerId, ChunkStream>,
) -> Result<Vec<(DetectorId, DetectionStream)>, Error> {
    todo!()
}

/// Spawns text generation detection tasks.
/// Returns a vec of detections.
#[instrument(skip_all)]
pub async fn text_generation_detections(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: &HashMap<DetectorId, DetectorParams>,
    prompt: String,
    generated_text: String,
) -> Result<Vec<(DetectorId, Detections)>, Error> {
    todo!()
}

/// Spawns text chat detection tasks.
/// Returns a vec of detections.
#[instrument(skip_all)]
pub async fn text_chat_detections(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: &HashMap<DetectorId, DetectorParams>,
    messages: Vec<openai::Message>,
) -> Result<Vec<(DetectorId, Detections)>, Error> {
    todo!()
}

/// Spawns text context detection tasks.
/// Returns a vec of detections.
#[instrument(skip_all)]
pub async fn text_context_detections(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: &HashMap<DetectorId, DetectorParams>,
    content: String,
    context_type: ContextType,
    context: Vec<String>,
) -> Result<Vec<(DetectorId, Detections)>, Error> {
    todo!()
}

// Client call helpers

/// Sends request to chunker client.
#[instrument(skip_all, fields(chunker_id))]
pub async fn chunk(
    ctx: Arc<Context>,
    chunker_id: ChunkerId,
    index: usize,
    text: String,
) -> Result<Chunks, Error> {
    if chunker_id == "whole_doc_chunker" {
        return Ok(vec![Chunk {
            index,
            start: 0,
            end: text.chars().count(),
            text,
        }]
        .into());
    }
    let client = ctx.clients.get_as::<ChunkerClient>(&chunker_id).unwrap();
    let request = ChunkerTokenizationTaskRequest { text };
    debug!(?request, "sending chunker request");
    let response = client
        .tokenization_task_predict(&chunker_id, request)
        .await
        .map_err(|error| Error::ChunkerRequestFailed {
            id: chunker_id.clone(),
            error,
        })?;
    debug!(?response, "received chunker response");
    Ok((index, response).into())
}

/// Sends request to text contents detector client.
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_text_contents(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detector_id: DetectorId,
    params: DetectorParams,
    chunks: Chunks,
) -> Result<Detections, Error> {
    let detector_id = detector_id.clone();
    let contents = chunks
        .into_iter()
        .map(|chunk| chunk.text)
        .collect::<Vec<_>>();
    if contents.is_empty() {
        return Ok(Detections::default());
    }
    let request = ContentAnalysisRequest::new(contents, params);
    debug!(?request, "sending detector request");
    let client = ctx
        .clients
        .get_as::<TextContentsDetectorClient>(&detector_id)
        .unwrap();
    let response = client
        .text_contents(&detector_id, request, headers)
        .await
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(?response, "received detector response");
    Ok(response.into())
}

/// Sends request to text generation detector client.
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_text_generation(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detector_id: DetectorId,
    params: DetectorParams,
    prompt: String,
    generated_text: String,
) -> Result<Detections, Error> {
    let detector_id = detector_id.clone();
    let request = GenerationDetectionRequest::new(prompt, generated_text, params);
    debug!(?request, "sending detector request");
    let client = ctx
        .clients
        .get_as::<TextGenerationDetectorClient>(&detector_id)
        .unwrap();
    let response = client
        .text_generation(&detector_id, request, headers)
        .await
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(?response, "received detector response");
    Ok(response.into())
}

/// Sends request to text chat detector client.
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_text_chat(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detector_id: DetectorId,
    params: DetectorParams,
    messages: Vec<openai::Message>,
) -> Result<Detections, Error> {
    let detector_id = detector_id.clone();
    let request = ChatDetectionRequest::new(messages, params);
    debug!(?request, "sending detector request");
    let client = ctx
        .clients
        .get_as::<TextChatDetectorClient>(&detector_id)
        .unwrap();
    let response = client
        .text_chat(&detector_id, request, headers)
        .await
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(?response, "received detector response");
    Ok(response.into())
}

/// Sends request to text context detector client.
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_text_context(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detector_id: DetectorId,
    params: DetectorParams,
    content: String,
    context_type: ContextType,
    context: Vec<String>,
) -> Result<Detections, Error> {
    let detector_id = detector_id.clone();
    let request = ContextDocsDetectionRequest::new(content, context_type, context, params.clone());
    debug!(?request, "sending detector request");
    let client = ctx
        .clients
        .get_as::<TextContextDocDetectorClient>(&detector_id)
        .unwrap();
    let response = client
        .text_context_doc(&detector_id, request, headers)
        .await
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(?response, "received detector response");
    Ok(response.into())
}

/// Sends request to openai chat completions client.
#[instrument(skip_all, fields(model_id))]
pub async fn chat_completions(
    ctx: Arc<Context>,
    headers: HeaderMap,
    mut request: openai::ChatCompletionsRequest,
) -> Result<openai::ChatCompletionsResponse, Error> {
    request.detectors = None;
    debug!("sending chat completions request");
    let model_id = request.model.clone();
    let client = ctx
        .clients
        .get_as::<OpenAiClient>("chat_generation")
        .unwrap();
    client
        .chat_completions(request, headers)
        .await
        .map_err(|error| Error::ChatGenerateRequestFailed {
            id: model_id,
            error,
        })
}

/// Sends tokenize request to generation client.
#[instrument(skip_all, fields(model_id))]
pub async fn tokenize(
    ctx: Arc<Context>,
    headers: HeaderMap,
    model_id: String,
    text: String,
) -> Result<(u32, Vec<String>), Error> {
    debug!("sending tokenize request");
    let client = ctx
        .clients
        .get_as::<GenerationClient>("generation")
        .unwrap();
    client
        .tokenize(model_id.clone(), text, headers)
        .await
        .map_err(|error| Error::TokenizeRequestFailed {
            id: model_id,
            error,
        })
}

/// Sends generate request to generation client.
#[instrument(skip_all, fields(model_id))]
pub async fn generate(
    ctx: Arc<Context>,
    headers: HeaderMap,
    model_id: String,
    text: String,
    params: Option<GenerateParams>,
) -> Result<GenerateResponse, Error> {
    debug!("sending generate request");
    let client = ctx
        .clients
        .get_as::<GenerationClient>("generation")
        .unwrap();
    client
        .generate(model_id.clone(), text, params, headers)
        .await
        .map_err(|error| Error::GenerateRequestFailed {
            id: model_id,
            error,
        })
}

/// Sends generate stream request to generation client.
#[instrument(skip_all, fields(model_id))]
pub async fn generate_stream(
    ctx: Arc<Context>,
    headers: HeaderMap,
    model_id: String,
    text: String,
    params: Option<GenerateParams>,
) -> Result<BoxStream<Result<GenerateStreamResponse, Error>>, Error> {
    debug!("sending generate stream request");
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

    #[tokio::test]
    async fn test_text_generation_detections() -> Result<(), Error> {
        todo!()
    }
}
