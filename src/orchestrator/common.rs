use std::{collections::HashMap, sync::Arc};

use futures::{StreamExt, TryStreamExt, stream};
use http::HeaderMap;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tracing::{debug, instrument};

use super::{Context, Error, types::*};
use crate::{
    clients::{
        GenerationClient, TextContentsDetectorClient,
        chunker::ChunkerClient,
        detector::{
            ChatDetectionRequest, ContentAnalysisRequest, ContextDocsDetectionRequest, ContextType,
            GenerationDetectionRequest, TextChatDetectorClient, TextContextDocDetectorClient,
            TextGenerationDetectorClient,
        },
        openai::{self, ChatCompletionsResponse, OpenAiClient},
    },
    models::{
        ClassifiedGeneratedTextResult as GenerateResponse, DetectorParams,
        GuardrailsTextGenerationParameters as GenerateParams,
    },
    pb::caikit::runtime::chunkers::{
        BidiStreamingChunkerTokenizationTaskRequest, ChunkerTokenizationTaskRequest,
    },
};

pub mod ext;
pub mod utils;
pub use utils::*;

// Processing tasks

/// Spawns chunk tasks. Returns a map of chunks.
#[instrument(skip_all)]
pub async fn chunks(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    inputs: Vec<(usize, String)>, // (offset, text)
) -> Result<HashMap<ChunkerId, Chunks>, Error> {
    let inputs = chunkers
        .iter()
        .flat_map(|chunker_id| {
            inputs
                .iter()
                .map(|(offset, text)| (chunker_id.clone(), *offset, text.clone()))
        })
        .collect::<Vec<_>>();
    let results = stream::iter(inputs)
        .map(|(chunker_id, offset, text)| {
            let ctx = ctx.clone();
            async move {
                let chunks = chunk(ctx, chunker_id.clone(), text)
                    .await?
                    .into_iter()
                    .map(|mut chunk| {
                        chunk.start += offset;
                        chunk.end += offset;
                        chunk
                    })
                    .collect();
                Ok::<_, Error>((chunker_id, chunks))
            }
        })
        .buffer_unordered(8)
        .try_collect::<HashMap<_, _>>()
        .await?;
    Ok(results)
}

/// Spawns chunk streaming tasks.
/// Returns a map of chunk broadcast channels.
#[instrument(skip_all)]
pub async fn chunk_streams(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    input_rx: InputReceiver,
) -> Result<HashMap<ChunkerId, broadcast::Sender<Result<Chunk, Error>>>, Error> {
    // Create input broadcast channel
    let input_stream = ReceiverStream::new(input_rx).boxed();
    let input_broadcast_tx = broadcast_stream(input_stream);

    let mut streams = Vec::with_capacity(chunkers.len());
    for chunker_id in chunkers {
        // Subscribe to input broadcast channel
        let input_broadcast_rx = input_broadcast_tx.subscribe();
        // Get chunk stream
        let chunk_stream =
            chunk_stream(ctx.clone(), chunker_id.clone(), input_broadcast_rx).await?;
        // Create chunk broadcast channel
        let chunk_broadcast_tx = broadcast_stream(chunk_stream);
        streams.push((chunker_id, chunk_broadcast_tx));
    }

    Ok(streams.into_iter().collect())
}

/// Spawns text contents detection tasks.
/// Returns a vec of detections.
#[instrument(skip_all)]
pub async fn text_contents_detections(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: HashMap<String, DetectorParams>,
    input_id: InputId,
    inputs: Vec<(usize, String)>,
) -> Result<Vec<(InputId, DetectorId, Detections)>, Error> {
    let chunkers = get_chunker_ids(&ctx.config, &detectors)?;
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
                let mut detections =
                    detect_text_contents(ctx, headers, detector_id.clone(), params, chunks)
                        .await?
                        .into_iter()
                        .filter(|detection| detection.score >= threshold)
                        .map(|mut detection| {
                            detection.detector_id = Some(detector_id.clone());
                            detection
                        })
                        .collect::<Detections>();
                detections.sort_by_key(|detection| detection.start);
                Ok::<_, Error>((input_id, detector_id, detections))
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
    detectors: &HashMap<String, DetectorParams>,
    input_id: InputId,
    input_rx: InputReceiver,
) -> Result<Vec<DetectionStream>, Error> {
    // Create chunk streams
    let chunkers = get_chunker_ids(&ctx.config, detectors)?;
    let chunk_streams = chunk_streams(ctx.clone(), chunkers, input_rx).await?;
    // Create detection streams
    let mut streams = Vec::with_capacity(detectors.len());
    for (detector_id, params) in detectors {
        let ctx = ctx.clone();
        let headers = headers.clone();
        let detector_id = detector_id.clone();
        let params = params.clone();
        let chunker_id = ctx.config.get_chunker_id(&detector_id).unwrap();
        // Subscribe to chunk broadcast channel
        let mut chunk_rx = chunk_streams.get(&chunker_id).unwrap().subscribe();
        // Create detection channel
        let (detection_tx, detection_rx) = mpsc::channel(32);
        // Spawn detection task
        tokio::spawn(async move {
            while let Ok(result) = chunk_rx.recv().await {
                match result {
                    Ok(chunk) => {
                        match detect_text_contents(
                            ctx.clone(),
                            headers.clone(),
                            detector_id.clone(),
                            params.clone(),
                            vec![chunk.clone()].into(),
                        )
                        .await
                        {
                            Ok(detections) => {
                                // Send to detection channel
                                let _ = detection_tx
                                    .send(Ok((input_id, detector_id.clone(), chunk, detections)))
                                    .await;
                            }
                            Err(error) => {
                                // Send error to detection channel
                                let _ = detection_tx.send(Err(error)).await;
                            }
                        }
                    }
                    Err(error) => {
                        // Send error to detection channel
                        let _ = detection_tx.send(Err(error)).await;
                    }
                }
            }
        });
        let detection_stream = ReceiverStream::new(detection_rx).boxed();
        streams.push(detection_stream);
    }
    Ok(streams)
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
    text: String,
) -> Result<Chunks, Error> {
    if chunker_id == "whole_doc_chunker" {
        return Ok(vec![Chunk {
            input_start_index: 0,
            input_end_index: 0,
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
    Ok(response.into())
}

/// Sends chunk stream request to chunker client.
#[instrument(skip_all, fields(chunker_id))]
pub async fn chunk_stream(
    ctx: Arc<Context>,
    chunker_id: ChunkerId,
    input_broadcast_rx: broadcast::Receiver<Result<(usize, String), Error>>,
) -> Result<ChunkStream, Error> {
    let client = ctx.clients.get_as::<ChunkerClient>(&chunker_id).unwrap();
    let input_stream = BroadcastStream::new(input_broadcast_rx)
        .map(|result| {
            let (index, text) = result.unwrap().unwrap(); // TODO: determine how to handle
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: text,
                input_index_stream: index as i64,
            }
        })
        .boxed();
    let output_stream = client
        .bidi_streaming_tokenization_task_predict(&chunker_id, input_stream)
        .await
        .map_err(|error| Error::ChunkerRequestFailed {
            id: chunker_id.clone(),
            error,
        })?
        .map_ok(Into::into)
        .map_err(move |error| Error::ChunkerRequestFailed {
            id: chunker_id.clone(),
            error,
        })
        .boxed();
    Ok(output_stream)
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
pub async fn chat_completion(
    ctx: Arc<Context>,
    headers: HeaderMap,
    mut request: openai::ChatCompletionsRequest,
) -> Result<openai::ChatCompletionsResponse, Error> {
    request.stream = false;
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

/// Sends request to openai chat completions client.
#[instrument(skip_all, fields(model_id))]
pub async fn chat_completion_stream(
    ctx: Arc<Context>,
    headers: HeaderMap,
    mut request: openai::ChatCompletionsRequest,
) -> Result<ChatCompletionStream, Error> {
    debug!("sending chat completions request");
    request.stream = true;
    request.detectors = None;
    let model_id = request.model.clone();
    let client = ctx
        .clients
        .get_as::<OpenAiClient>("chat_generation")
        .unwrap();
    let response = client
        .chat_completions(request, headers)
        .await
        .map_err(|error| Error::ChatGenerateRequestFailed {
            id: model_id,
            error,
        })?;
    let stream = match response {
        ChatCompletionsResponse::Streaming(rx) => ReceiverStream::new(rx),
        ChatCompletionsResponse::Unary(_) => unimplemented!(),
    }
    .enumerate()
    .map(|(index, result)| match result {
        Ok(Some(completion)) => Ok(Some((index, completion))),
        Ok(None) => Ok(None),
        Err(error) => Err(error),
    })
    .boxed();
    Ok(stream)
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
) -> Result<GenerationStream, Error> {
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
