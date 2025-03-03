//! Processing tasks
use std::{collections::HashMap, sync::Arc};

use futures::{StreamExt, TryStreamExt, stream};
use http::HeaderMap;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tracing::instrument;

use super::{client::*, utils::*};
use crate::{
    clients::{detector::ContextType, openai},
    models::DetectorParams,
    orchestrator::{Context, Error, types::*},
};

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
) -> Result<(InputId, Detections), Error> {
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
                let detections =
                    detect_text_contents(ctx, headers, detector_id.clone(), params, chunks)
                        .await?
                        .into_iter()
                        .filter(|detection| detection.score >= threshold)
                        .map(|mut detection| {
                            detection.detector_id = Some(detector_id.clone());
                            detection
                        })
                        .collect::<Detections>();
                Ok::<_, Error>(detections)
            }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await?;
    let mut detections = results.into_iter().flatten().collect::<Detections>();
    detections.sort_by_key(|detection| detection.start);
    Ok((input_id, detections))
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
    input_id: InputId,
    prompt: String,
    generated_text: String,
) -> Result<(InputId, Detections), Error> {
    let inputs = detectors
        .iter()
        .map(|(detector_id, params)| {
            Ok::<_, Error>((
                detector_id.clone(),
                params.clone(),
                prompt.clone(),
                generated_text.clone(),
            ))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let results = stream::iter(inputs)
        .map(|(detector_id, mut params, prompt, generated_text)| {
            let ctx = ctx.clone();
            let headers = headers.clone();
            let threshold = params.pop_threshold().unwrap_or_default();
            async move {
                let detections = detect_text_generation(
                    ctx,
                    headers,
                    detector_id.clone(),
                    params,
                    prompt,
                    generated_text,
                )
                .await?
                .into_iter()
                .filter(|detection| detection.score >= threshold)
                .map(|mut detection| {
                    detection.detector_id = Some(detector_id.clone());
                    detection
                })
                .collect::<Detections>();
                Ok::<_, Error>(detections)
            }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await?;
    let detections = results.into_iter().flatten().collect::<Detections>();
    Ok((input_id, detections))
}

/// Spawns text chat detection tasks.
/// Returns a vec of detections.
#[instrument(skip_all)]
pub async fn text_chat_detections(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: &HashMap<DetectorId, DetectorParams>,
    input_id: InputId,
    messages: Vec<openai::Message>,
) -> Result<(InputId, Detections), Error> {
    let inputs = detectors
        .iter()
        .map(|(detector_id, params)| {
            Ok::<_, Error>((detector_id.clone(), params.clone(), messages.clone()))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let results = stream::iter(inputs)
        .map(|(detector_id, mut params, messages)| {
            let ctx = ctx.clone();
            let headers = headers.clone();
            let threshold = params.pop_threshold().unwrap_or_default();
            async move {
                let detections =
                    detect_text_chat(ctx, headers, detector_id.clone(), params, messages)
                        .await?
                        .into_iter()
                        .filter(|detection| detection.score >= threshold)
                        .map(|mut detection| {
                            detection.detector_id = Some(detector_id.clone());
                            detection
                        })
                        .collect::<Detections>();
                Ok::<_, Error>(detections)
            }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await?;
    let detections = results.into_iter().flatten().collect::<Detections>();
    Ok((input_id, detections))
}

/// Spawns text context detection tasks.
/// Returns a vec of detections.
#[instrument(skip_all)]
pub async fn text_context_detections(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: &HashMap<DetectorId, DetectorParams>,
    input_id: InputId,
    content: String,
    context_type: ContextType,
    context: Vec<String>,
) -> Result<(InputId, Detections), Error> {
    let inputs = detectors
        .iter()
        .map(|(detector_id, params)| {
            Ok::<_, Error>((
                detector_id.clone(),
                params.clone(),
                content.clone(),
                context_type.clone(),
                context.clone(),
            ))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let results = stream::iter(inputs)
        .map(
            |(detector_id, mut params, content, context_type, context)| {
                let ctx = ctx.clone();
                let headers = headers.clone();
                let threshold = params.pop_threshold().unwrap_or_default();
                async move {
                    let detections = detect_text_context(
                        ctx,
                        headers,
                        detector_id.clone(),
                        params,
                        content,
                        context_type,
                        context,
                    )
                    .await?
                    .into_iter()
                    .filter(|detection| detection.score >= threshold)
                    .map(|mut detection| {
                        detection.detector_id = Some(detector_id.clone());
                        detection
                    })
                    .collect::<Detections>();
                    Ok::<_, Error>(detections)
                }
            },
        )
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await?;
    let detections = results.into_iter().flatten().collect::<Detections>();
    Ok((input_id, detections))
}
