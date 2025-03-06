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
//! Processing tasks
use std::{collections::HashMap, sync::Arc};

use futures::{stream, StreamExt, TryStreamExt};
use http::HeaderMap;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, instrument};

use super::{client::*, utils::*};
use crate::{
    clients::{
        chunker::{ChunkerClient, DEFAULT_CHUNKER_ID},
        detector::{
            ContextType, TextChatDetectorClient, TextContextDocDetectorClient,
            TextGenerationDetectorClient,
        },
        openai, TextContentsDetectorClient,
    },
    models::DetectorParams,
    orchestrator::{types::*, Context, Error},
};

/// Spawns chunk tasks. Returns a map of chunks.
#[instrument(skip_all)]
pub async fn chunks(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    inputs: Vec<(usize, String)>, // (offset, text)
) -> Result<HashMap<ChunkerId, Chunks>, Error> {
    fn whole_doc_chunk(offset: usize, text: String) -> Chunks {
        vec![Chunk {
            start: offset,
            end: text.chars().count() + offset,
            text,
            ..Default::default()
        }]
        .into()
    }
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
                if chunker_id == DEFAULT_CHUNKER_ID {
                    debug!("using whole doc chunker");
                    // Return single chunk
                    return Ok((chunker_id, whole_doc_chunk(offset, text)));
                }
                let client = ctx.clients.get_as::<ChunkerClient>(&chunker_id).unwrap();
                let chunks = chunk(client, chunker_id.clone(), text)
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
    // TODO: Drop support for this as it collects the entire input stream
    fn whole_doc_chunk_stream(
        mut input_broadcast_rx: broadcast::Receiver<Result<(usize, String), Error>>,
    ) -> Result<ChunkStream, Error> {
        // Create output channel
        let (output_tx, output_rx) = mpsc::channel(1);
        // Spawn task to collect input channel
        tokio::spawn(async move {
            // Collect input channel
            // Alternatively, wrap receiver in BroadcastStream and collect() via StreamExt
            let mut inputs = Vec::new();
            while let Ok(input) = input_broadcast_rx.recv().await.unwrap() {
                inputs.push(input);
            }
            // Build chunk
            let (indices, text): (Vec<_>, Vec<_>) = inputs.into_iter().unzip();
            let text = text.concat();
            let chunk = Chunk {
                input_start_index: 0,
                input_end_index: indices.last().copied().unwrap_or_default(),
                start: 0,
                end: text.chars().count(),
                text,
            };
            // Send chunk to output channel
            let _ = output_tx.send(Ok::<_, Error>(chunk)).await;
        });

        Ok::<_, Error>(ReceiverStream::new(output_rx).boxed())
    }

    // Create input broadcast channel
    let input_stream = ReceiverStream::new(input_rx).boxed();
    let input_broadcast_tx = broadcast_stream(input_stream);

    // Create chunk broadcast channels for each chunker
    let mut streams = Vec::with_capacity(chunkers.len());
    for chunker_id in chunkers {
        // Subscribe to input broadcast channel
        let input_broadcast_rx = input_broadcast_tx.subscribe();
        // Open chunk stream
        let chunk_stream = if chunker_id == DEFAULT_CHUNKER_ID {
            debug!("using whole doc chunker");
            whole_doc_chunk_stream(input_broadcast_rx)
        } else {
            let client = ctx.clients.get_as::<ChunkerClient>(&chunker_id).unwrap();
            chunk_stream(client, chunker_id.clone(), input_broadcast_rx).await
        }?;
        // Create chunk broadcast channel
        let chunk_broadcast_tx = broadcast_stream(chunk_stream);
        streams.push((chunker_id, chunk_broadcast_tx));
    }

    // Return a map of chunker_id->chunker_broadcast_tx
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
    let chunkers = get_chunker_ids(&ctx, &detectors)?;
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
                let client = ctx
                    .clients
                    .get_as::<TextContentsDetectorClient>(&detector_id)
                    .unwrap();
                let detections =
                    detect_text_contents(client, headers, detector_id.clone(), params, chunks)
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
    let chunkers = get_chunker_ids(&ctx, detectors)?;
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
                        let client = ctx
                            .clients
                            .get_as::<TextContentsDetectorClient>(&detector_id)
                            .unwrap();
                        match detect_text_contents(
                            client,
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
                let client = ctx
                    .clients
                    .get_as::<TextGenerationDetectorClient>(&detector_id)
                    .unwrap();
                let detections = detect_text_generation(
                    client,
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
                let client = ctx
                    .clients
                    .get_as::<TextChatDetectorClient>(&detector_id)
                    .unwrap();
                let detections =
                    detect_text_chat(client, headers, detector_id.clone(), params, messages)
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
                    let client = ctx
                        .clients
                        .get_as::<TextContextDocDetectorClient>(&detector_id)
                        .unwrap();
                    let detections = detect_text_context(
                        client,
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

/// Fans-out a stream to a broadcast channel.
pub fn broadcast_stream<T>(mut stream: BoxStream<T>) -> broadcast::Sender<T>
where
    T: Clone + Send + 'static,
{
    let (broadcast_tx, _) = broadcast::channel(32);
    tokio::spawn({
        let broadcast_tx = broadcast_tx.clone();
        async move {
            while let Some(msg) = stream.next().await {
                let _ = broadcast_tx.send(msg);
            }
        }
    });
    broadcast_tx
}
