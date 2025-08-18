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

use futures::{StreamExt, TryStreamExt, future::try_join_all, stream};
use http::HeaderMap;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{Instrument, debug, instrument};

use super::{client::*, utils::*};
use crate::{
    clients::{
        chunker::{ChunkerClient, DEFAULT_CHUNKER_ID},
        detector::{ContextType, DetectorClient},
        openai,
    },
    models::DetectorParams,
    orchestrator::{Context, Error, types::*},
};

/// Spawns chunk tasks. Returns a map of chunks.
pub async fn chunks(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    inputs: Vec<(usize, String)>, // (offset, text)
) -> Result<HashMap<ChunkerId, Chunks>, Error> {
    if inputs.is_empty() {
        return Ok(HashMap::default());
    }
    let tasks = chunkers
        .into_iter()
        .map(|chunker_id| {
            let ctx = ctx.clone();
            let inputs = inputs.clone();
            // Spawn task for chunker
            // Chunkers are processed in-parallel
            tokio::spawn(
                async move {
                    // Send concurrent requests for inputs
                    let chunks = stream::iter(inputs)
                        .map(|(offset, text)| {
                            let ctx = ctx.clone();
                            let chunker_id = chunker_id.clone();
                            async move {
                                if chunker_id == DEFAULT_CHUNKER_ID {
                                    debug!("using whole doc chunker");
                                    // Return single chunk
                                    return Ok(whole_doc_chunk(offset, text));
                                }
                                let client = ctx
                                    .clients
                                    .get::<ChunkerClient>(&chunker_id)
                                    .ok_or_else(|| Error::ChunkerNotFound(chunker_id.clone()))?;
                                let chunks = chunk(client, chunker_id.clone(), text)
                                    .await?
                                    .into_iter()
                                    .map(|mut chunk| {
                                        chunk.start += offset;
                                        chunk.end += offset;
                                        chunk
                                    })
                                    .collect::<Chunks>();
                                Ok::<_, Error>(chunks)
                            }
                            .in_current_span()
                        })
                        .buffer_unordered(ctx.config.chunker_concurrent_requests)
                        .try_collect::<Vec<_>>()
                        .await?
                        .into_iter()
                        .flatten()
                        .collect::<Chunks>();
                    Ok::<(ChunkerId, Chunks), Error>((chunker_id, chunks))
                }
                .in_current_span(),
            )
        })
        .collect::<Vec<_>>();
    let chunk_map = try_join_all(tasks)
        .await?
        .into_iter()
        .collect::<Result<HashMap<_, _>, Error>>()?;
    Ok(chunk_map)
}

/// Spawns chunk streaming tasks.
/// Returns a map of chunk broadcast channels.
pub async fn chunk_streams(
    ctx: Arc<Context>,
    chunkers: Vec<ChunkerId>,
    input_rx: mpsc::Receiver<Result<(usize, String), Error>>, // (message_index, text)
) -> Result<HashMap<ChunkerId, broadcast::Sender<Result<Chunk, Error>>>, Error> {
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
            // TODO: drop support for this as it collects the stream
            whole_doc_chunk_stream(input_broadcast_rx)
        } else {
            let client = ctx
                .clients
                .get::<ChunkerClient>(&chunker_id)
                .ok_or_else(|| Error::ChunkerNotFound(chunker_id.clone()))?;
            chunk_stream(client, chunker_id.clone(), input_broadcast_rx).await
        }?;
        // Create chunk broadcast channel
        let chunk_broadcast_tx = broadcast_stream(chunk_stream);
        streams.push((chunker_id, chunk_broadcast_tx));
    }

    // Return a map of chunker_id->chunker_broadcast_tx
    Ok(streams.into_iter().collect())
}

fn whole_doc_chunk(offset: usize, text: String) -> Chunks {
    vec![Chunk {
        start: offset,
        end: text.chars().count() + offset,
        text,
        ..Default::default()
    }]
    .into()
}

fn whole_doc_chunk_stream(
    mut input_broadcast_rx: broadcast::Receiver<Result<(usize, String), Error>>,
) -> Result<ChunkStream, Error> {
    // Create output channel
    let (output_tx, output_rx) = mpsc::channel(1);
    // Spawn task to collect input channel
    tokio::spawn(
        async move {
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
        }
        .in_current_span(),
    );

    Ok::<_, Error>(ReceiverStream::new(output_rx).boxed())
}

/// Spawns text contents detection tasks.
/// Returns a vec of detections.
#[instrument(skip_all)]
pub async fn text_contents_detections(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: HashMap<String, DetectorParams>,
    inputs: Vec<(usize, String)>,
) -> Result<Vec<Detection>, Error> {
    let chunkers = get_chunker_ids(&ctx, &detectors)?;
    let chunk_map = chunks(ctx.clone(), chunkers, inputs).await?;
    let inputs = detectors
        .iter()
        .map(|(detector_id, params)| {
            let config = ctx
                .config
                .detector(detector_id)
                .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?;
            let chunks = chunk_map.get(&config.chunker_id).unwrap().clone();
            Ok::<_, Error>((detector_id.clone(), params.clone(), chunks))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    // Send concurrent requests for inputs
    let results = stream::iter(inputs)
        .map(|(detector_id, mut params, chunks)| {
            let ctx = ctx.clone();
            let headers = headers.clone();
            let default_threshold = ctx.config.detector(&detector_id).unwrap().default_threshold;
            let threshold = params.pop_threshold().unwrap_or(default_threshold);
            async move {
                let client = ctx.clients.get::<DetectorClient>(&detector_id).unwrap();
                let detections = detect_text_contents(
                    client,
                    headers,
                    detector_id.clone(),
                    params,
                    chunks.clone(),
                    true,
                )
                .await?
                .into_iter()
                .filter(|detection| detection.score >= threshold)
                .collect::<Vec<_>>();
                Ok::<_, Error>(detections)
            }
            .in_current_span()
        })
        .buffer_unordered(ctx.config.detector_concurrent_requests)
        .try_collect::<Vec<_>>()
        .await?;
    let mut detections = results.into_iter().flatten().collect::<Vec<_>>();
    detections.sort_by_key(|detection| detection.start);
    Ok(detections)
}

/// Spawns text contents detection stream tasks.
/// Returns a vec of detection streams.
#[instrument(skip_all)]
pub async fn text_contents_detection_streams(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: HashMap<String, DetectorParams>,
    input_id: u32,
    input_rx: mpsc::Receiver<Result<(usize, String), Error>>, // (message_index, text)
) -> Result<Vec<DetectionStream>, Error> {
    // Create chunk streams
    let chunkers = get_chunker_ids(&ctx, &detectors)?;
    let chunk_stream_map = chunk_streams(ctx.clone(), chunkers, input_rx).await?;
    // Create detection streams
    let mut streams = Vec::with_capacity(detectors.len());
    for (detector_id, mut params) in detectors {
        let ctx = ctx.clone();
        let headers = headers.clone();
        let default_threshold = ctx.config.detector(&detector_id).unwrap().default_threshold;
        let threshold = params.pop_threshold().unwrap_or(default_threshold);
        let chunker_id = ctx.config.get_chunker_id(&detector_id).unwrap();
        // Subscribe to chunk broadcast channel
        let mut chunk_rx = chunk_stream_map.get(&chunker_id).unwrap().subscribe();
        // Create detection channel
        let (detection_tx, detection_rx) = mpsc::channel(128);
        // Spawn detection task
        tokio::spawn(
            async move {
                while let Ok(result) = chunk_rx.recv().await {
                    match result {
                        Ok(chunk) => {
                            let client = ctx.clients.get::<DetectorClient>(&detector_id).unwrap();
                            match detect_text_contents(
                                client,
                                headers.clone(),
                                detector_id.clone(),
                                params.clone(),
                                vec![chunk.clone()].into(),
                                false,
                            )
                            .await
                            {
                                Ok(detections) => {
                                    // Apply threshold
                                    let detections = detections
                                        .into_iter()
                                        .filter(|detection| detection.score >= threshold)
                                        .collect::<Vec<_>>();
                                    // Send to detection channel
                                    let _ =
                                        detection_tx.send(Ok((input_id, chunk, detections))).await;
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
            }
            .in_current_span(),
        );
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
    detectors: HashMap<DetectorId, DetectorParams>,
    prompt: String,
    generated_text: String,
) -> Result<Vec<Detection>, Error> {
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
    // Send concurrent requests for inputs
    let results = stream::iter(inputs)
        .map(|(detector_id, mut params, prompt, generated_text)| {
            let ctx = ctx.clone();
            let headers = headers.clone();
            let default_threshold = ctx.config.detector(&detector_id).unwrap().default_threshold;
            let threshold = params.pop_threshold().unwrap_or(default_threshold);
            async move {
                let client = ctx.clients.get::<DetectorClient>(&detector_id).unwrap();
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
                .collect::<Vec<_>>();
                Ok::<_, Error>(detections)
            }
            .in_current_span()
        })
        .buffer_unordered(ctx.config.detector_concurrent_requests)
        .try_collect::<Vec<_>>()
        .await?;
    let detections = results.into_iter().flatten().collect::<Vec<_>>();
    Ok(detections)
}

/// Spawns text chat detection tasks.
/// Returns a vec of detections.
#[instrument(skip_all)]
pub async fn text_chat_detections(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: HashMap<DetectorId, DetectorParams>,
    messages: Vec<openai::Message>,
    tools: Vec<openai::Tool>,
) -> Result<Vec<Detection>, Error> {
    let inputs = detectors
        .iter()
        .map(|(detector_id, params)| {
            Ok::<_, Error>((
                detector_id.clone(),
                params.clone(),
                messages.clone(),
                tools.clone(),
            ))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    // Send concurrent requests for inputs
    let results = stream::iter(inputs)
        .map(|(detector_id, mut params, messages, tools)| {
            let ctx = ctx.clone();
            let headers = headers.clone();
            let default_threshold = ctx.config.detector(&detector_id).unwrap().default_threshold;
            let threshold = params.pop_threshold().unwrap_or(default_threshold);
            async move {
                let client = ctx.clients.get::<DetectorClient>(&detector_id).unwrap();
                let detections = detect_text_chat(
                    client,
                    headers,
                    detector_id.clone(),
                    params,
                    messages,
                    tools,
                )
                .await?
                .into_iter()
                .filter(|detection| detection.score >= threshold)
                .collect::<Vec<_>>();
                Ok::<_, Error>(detections)
            }
            .in_current_span()
        })
        .buffer_unordered(ctx.config.detector_concurrent_requests)
        .try_collect::<Vec<_>>()
        .await?;
    let detections = results.into_iter().flatten().collect::<Vec<_>>();
    Ok(detections)
}

/// Spawns text context detection tasks.
/// Returns a vec of detections.
#[instrument(skip_all)]
pub async fn text_context_detections(
    ctx: Arc<Context>,
    headers: HeaderMap,
    detectors: HashMap<DetectorId, DetectorParams>,
    content: String,
    context_type: ContextType,
    context: Vec<String>,
) -> Result<Vec<Detection>, Error> {
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
    // Send concurrent requests for inputs
    let results = stream::iter(inputs)
        .map(
            |(detector_id, mut params, content, context_type, context)| {
                let ctx = ctx.clone();
                let headers = headers.clone();
                let default_threshold =
                    ctx.config.detector(&detector_id).unwrap().default_threshold;
                let threshold = params.pop_threshold().unwrap_or(default_threshold);
                async move {
                    let client = ctx.clients.get::<DetectorClient>(&detector_id).unwrap();
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
                    .collect::<Vec<_>>();
                    Ok::<_, Error>(detections)
                }
                .in_current_span()
            },
        )
        .buffer_unordered(ctx.config.detector_concurrent_requests)
        .try_collect::<Vec<_>>()
        .await?;
    let detections = results.into_iter().flatten().collect::<Vec<_>>();
    Ok(detections)
}

/// Fans-out a stream to a broadcast channel.
pub fn broadcast_stream<T>(mut stream: BoxStream<T>) -> broadcast::Sender<T>
where
    T: Clone + Send + 'static,
{
    let (broadcast_tx, _) = broadcast::channel(128);
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

#[cfg(test)]
mod test {

    use mocktail::prelude::*;
    use tokio::sync::OnceCell;

    use super::*;
    use crate::{
        clients::detector::{ContentAnalysisRequest, ContentAnalysisResponse},
        config::OrchestratorConfig,
        models::Metadata,
        orchestrator::create_clients,
        pb::{
            caikit::runtime::chunkers::{
                BidiStreamingChunkerTokenizationTaskRequest, ChunkerTokenizationTaskRequest,
            },
            caikit_data_model::nlp::{ChunkerTokenizationStreamResult, Token, TokenizationResults},
        },
    };

    static CONTEXT: OnceCell<Arc<Context>> = OnceCell::const_new();

    const CHUNKER_PATH: &str =
        "/caikit.runtime.Chunkers.ChunkersService/ChunkerTokenizationTaskPredict";
    const CHUNKER_STREAMING_PATH: &str =
        "/caikit.runtime.Chunkers.ChunkersService/BidiStreamingChunkerTokenizationTaskPredict";
    const TEXT_CONTENTS_DETECTOR_PATH: &str = "/api/v1/text/contents";

    const TEXT1: &str = "Lorem ipsum dolor sit amet, consectetuer adipiscing elit. \
        Aenean commodo ligula eget dolor. Cum sociis natoque \
        penatibus et magnis dis parturient montes, nascetur ridiculus mus.";
    const TEXT2: &str = "Qui reprehenderit aspernatur est unde autem et corporis animi hic \
        autem distinctio cum dolore fugit hic nihil vitae. Quo magni voluptatem et \
        vitae maxime est voluptatem itaque. ";

    async fn init_context() -> Arc<Context> {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Create sentence_chunker
        let mut mocks = MockSet::new();
        mocks.mock(|when, then| {
            when.path(CHUNKER_PATH)
                .pb(ChunkerTokenizationTaskRequest { text: TEXT1.into() });
            then.pb(TokenizationResults {
                results: vec![
                    Token { start: 0, end: 57, text: "Lorem ipsum dolor sit amet, consectetuer adipiscing elit.".into() },
                    Token { start: 57, end: 92, text: " Aenean commodo ligula eget dolor.".into() },
                    Token { start: 92, end: 179, text: " Cum sociis natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus.".into() }
                ],
                token_count: 25,
            });
        });
        mocks.mock(|when, then| {
            when.path(CHUNKER_PATH)
                .pb(ChunkerTokenizationTaskRequest { text: TEXT2.into() });
            then.pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 139,
                        text: "Qui reprehenderit aspernatur est unde autem et corporis animi hic \
                        autem distinctio cum dolore fugit hic nihil vitae."
                            .into(),
                    },
                    Token {
                        start: 139,
                        end: 201,
                        text: " Quo magni voluptatem et vitae maxime est voluptatem itaque. "
                            .into(),
                    },
                ],
                token_count: 27,
            });
        });
        mocks.mock(|when, then| {
            when.path(CHUNKER_STREAMING_PATH).pb_stream([
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "Lorem ipsum".into(),
                    input_index_stream: 0,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: " dolor sit amet, ".into(),
                    input_index_stream: 1,
                },
                BidiStreamingChunkerTokenizationTaskRequest {
                    text_stream: "consectetuer adipiscing elit.".into(),
                    input_index_stream: 2,
                },
            ]);
            then.pb_stream([ChunkerTokenizationStreamResult {
                results: vec![Token {
                    start: 0,
                    end: 57,
                    text: "Lorem ipsum dolor sit amet, consectetuer adipiscing elit.".into(),
                }],
                token_count: 8,
                processed_index: 57,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 2,
            }]);
        });
        let sentence_chunker_server = MockServer::new_grpc("sentence_chunker").with_mocks(mocks);
        sentence_chunker_server.start().await.unwrap();

        // Create whole_doc_chunker
        let mut mocks = MockSet::new();
        mocks.mock(|when, then| {
            when.path(CHUNKER_PATH)
                .pb(ChunkerTokenizationTaskRequest { text: TEXT1.into() });
            then.pb(TokenizationResults {
                results: vec![Token {
                    start: 0,
                    end: 179,
                    text: TEXT1.into(),
                }],
                token_count: 25,
            });
        });
        let whole_doc_chunker_server = MockServer::new_grpc("whole_doc_chunker").with_mocks(mocks);
        whole_doc_chunker_server.start().await.unwrap();

        // Create error chunker
        let mut mocks = MockSet::new();
        mocks.mock(|when, then| {
            when.path(CHUNKER_PATH);
            then.internal_server_error();
        });
        let error_chunker_server = MockServer::new_grpc("error_chunker").with_mocks(mocks);
        error_chunker_server.start().await.unwrap();

        // Create fake detector
        let mut mocks = MockSet::new();
        mocks.mock(|when, then| {
            when.post()
                .path(TEXT_CONTENTS_DETECTOR_PATH)
                .json(ContentAnalysisRequest {
                    contents: vec![
                        "Lorem ipsum dolor sit amet, consectetuer adipiscing elit.".into(),
                        " Aenean commodo ligula eget dolor.".into(),
                        " Cum sociis natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus.".into(),
                    ],
                    detector_params: Default::default(),
                });
            then.json(vec![vec![ContentAnalysisResponse {
                start: 5,
                end: 9,
                text: "amet".into(),
                detection: "nothing".into(),
                detection_type: "fake".into(),
                detector_id: None,
                score: 0.2,
                evidence: None,
                metadata: Metadata::new(),
            }]]);
        });
        mocks.mock(|when, then| {
            when.post()
                .path(TEXT_CONTENTS_DETECTOR_PATH)
                .json(ContentAnalysisRequest {
                    contents: vec![
                        "Lorem ipsum dolor sit amet, consectetuer adipiscing elit.".into(),
                    ],
                    detector_params: Default::default(),
                });
            then.json(vec![vec![ContentAnalysisResponse {
                start: 5,
                end: 9,
                text: "amet".into(),
                detection: "nothing".into(),
                detection_type: "fake".into(),
                detector_id: None,
                score: 0.2,
                evidence: None,
                metadata: Metadata::new(),
            }]]);
        });

        let fake_detector_server = MockServer::new_http("fake_detector").with_mocks(mocks);
        fake_detector_server.start().await.unwrap();

        let mut config = OrchestratorConfig::default();
        configure_mock_servers(
            &mut config,
            None,
            None,
            Some(vec![&fake_detector_server]),
            Some(vec![
                &sentence_chunker_server,
                &whole_doc_chunker_server,
                &error_chunker_server,
            ]),
        );
        // Set chunker_id for detectors
        if let Some(config) = config.detectors.get_mut("fake_detector") {
            config.chunker_id = "sentence_chunker".into();
        }

        // Create clients
        let clients = create_clients(&config).await.unwrap();

        Arc::new(Context::new(config, clients))
    }

    #[test_log::test(tokio::test)]
    async fn tests() -> Result<(), Error> {
        test_chunks().await?;
        test_text_contents_detections().await?;
        test_broadcast_stream().await?;
        test_chunk_streams().await?;
        test_text_contents_detection_streams().await?;
        Ok(())
    }

    async fn test_chunks() -> Result<(), Error> {
        let ctx = CONTEXT.get_or_init(init_context).await;

        // Single input with sentence chunker and whole doc chunker
        let chunk_map = chunks(
            ctx.clone(),
            vec!["sentence_chunker".into(), "whole_doc_chunker".into()],
            vec![(0, TEXT1.to_string())],
        )
        .await?;

        assert_eq!(chunk_map.len(), 2, "chunk map length should be 2");
        assert!(
            chunk_map
                .get("sentence_chunker")
                .is_some_and(|c| c.len() == 3),
            "sentence_chunker should have 3 chunks"
        );
        assert!(
            chunk_map
                .get("whole_doc_chunker")
                .is_some_and(|c| c.len() == 1),
            "whole_doc_chunker should have 1 chunk"
        );

        // Multiple inputs with sentence chunker
        let chunk_map = chunks(
            ctx.clone(),
            vec!["sentence_chunker".into()],
            vec![(0, TEXT1.to_string()), (0, TEXT2.to_string())],
        )
        .await?;
        assert_eq!(chunk_map.len(), 1, "chunk map length should be 1");
        assert!(
            chunk_map
                .get("sentence_chunker")
                .is_some_and(|c| c.len() == 5),
            "sentence_chunker should have 5 chunks"
        );

        // Chunker does not exist
        let result = chunks(
            ctx.clone(),
            vec!["does_not_exist".into()],
            vec![(0, TEXT1.to_string())],
        )
        .await;
        assert!(
            result.is_err_and(|e| matches!(e, Error::ChunkerNotFound(_))),
            "should return chunker not found error"
        );

        // Chunker server error
        let result = chunks(
            ctx.clone(),
            vec!["error_chunker".into()],
            vec![(0, TEXT1.to_string())],
        )
        .await;
        assert!(
            result.is_err_and(|e| {
                match e {
                    Error::ChunkerRequestFailed { error, .. } => {
                        error.status_code().is_server_error()
                    }
                    _ => false,
                }
            }),
            "should return chunker request failed error"
        );

        // Empty inputs
        let chunk_map = chunks(ctx.clone(), vec!["sentence_chunker".into()], vec![]).await?;
        assert!(chunk_map.is_empty(), "chunk map should be empty");

        // With mask offsets
        let chunk_map = chunks(
            ctx.clone(),
            vec!["sentence_chunker".into()],
            vec![(5, TEXT1.to_string())],
        )
        .await?;
        assert_eq!(chunk_map.len(), 1, "chunk map length should be 1");
        assert!(
            chunk_map
                .get("sentence_chunker")
                .is_some_and(|c| c.len() == 3),
            "should have 3 chunks"
        );
        assert_eq!(
            chunk_map.get("sentence_chunker").unwrap()[0].start,
            5,
            "chunk 1 start index should be equal to the offset 5"
        );

        Ok(())
    }

    async fn test_text_contents_detections() -> Result<(), Error> {
        let ctx = CONTEXT.get_or_init(init_context).await;

        // Single detector
        let mut detector_params = DetectorParams::new();
        detector_params.insert("threshold".to_string(), 0.2.into());
        let detectors = HashMap::from([("fake_detector".to_string(), detector_params)]);
        let detections = text_contents_detections(
            ctx.clone(),
            HeaderMap::default(),
            detectors,
            vec![(0, TEXT1.to_string())],
        )
        .await?;
        assert_eq!(detections.len(), 1, "should have 1 detection");

        // Single detector, below threshold
        let mut detector_params = DetectorParams::new();
        detector_params.insert("threshold".to_string(), 0.4.into());
        let detectors = HashMap::from([("fake_detector".to_string(), detector_params)]);
        let detections = text_contents_detections(
            ctx.clone(),
            HeaderMap::default(),
            detectors,
            vec![(0, TEXT1.to_string())],
        )
        .await?;
        assert!(detections.is_empty(), "should have no detections");

        // Detector does not exist
        let detectors = HashMap::from([("does_not_exist".to_string(), DetectorParams::new())]);
        let result = text_contents_detections(
            ctx.clone(),
            HeaderMap::default(),
            detectors,
            vec![(0, TEXT1.to_string())],
        )
        .await;
        assert!(
            result.is_err_and(|e| matches!(e, Error::DetectorNotFound(_))),
            "should return detector not found error"
        );

        // TODO: add more cases

        Ok(())
    }

    async fn test_broadcast_stream() -> Result<(), Error> {
        let (tx, rx) = mpsc::channel(32);
        let stream = ReceiverStream::new(rx).boxed();
        let broadcast_tx = broadcast_stream(stream);
        let mut broadcast_rx = broadcast_tx.subscribe();
        drop(broadcast_tx);

        // Spawn task to send values
        tokio::spawn(async move {
            for value in 0..10 {
                let _ = tx.send(value).await;
            }
        });

        // Consume values from broadcast receiver
        let mut values = Vec::with_capacity(10);
        while let Ok(value) = broadcast_rx.recv().await {
            values.push(value);
        }
        assert_eq!(values, (0..10).collect::<Vec<_>>());

        Ok(())
    }

    async fn test_chunk_streams() -> Result<(), Error> {
        let ctx = CONTEXT.get_or_init(init_context).await;

        // Create input channel
        let (input_tx, input_rx) = mpsc::channel(4);
        // Create inputs
        let inputs = vec![
            (0, "Lorem ipsum".into()),
            (1, " dolor sit amet, ".into()),
            (2, "consectetuer adipiscing elit.".into()),
        ];

        let chunk_stream_map =
            chunk_streams(ctx.clone(), vec!["sentence_chunker".into()], input_rx).await?;

        let mut chunk_broadcast_rx = chunk_stream_map
            .get("sentence_chunker")
            .unwrap()
            .subscribe();
        drop(chunk_stream_map);

        // Spawn task to send inputs to input channel
        tokio::spawn(async move {
            for input in inputs {
                let _ = input_tx.send(Ok(input)).await;
            }
        });

        let mut chunks = Vec::with_capacity(1);
        while let Ok(Ok(chunk)) = chunk_broadcast_rx.recv().await {
            chunks.push(chunk);
        }

        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0],
            Chunk {
                input_start_index: 0,
                input_end_index: 2,
                start: 0,
                end: 57,
                text: "Lorem ipsum dolor sit amet, consectetuer adipiscing elit.".into(),
            }
        );

        Ok(())
    }

    async fn test_text_contents_detection_streams() -> Result<(), Error> {
        let ctx = CONTEXT.get_or_init(init_context).await;

        // Create input channel
        let (input_tx, input_rx) = mpsc::channel(4);
        // Create inputs
        let inputs = vec![
            (0, "Lorem ipsum".into()),
            (1, " dolor sit amet, ".into()),
            (2, "consectetuer adipiscing elit.".into()),
        ];

        let mut detector_params = DetectorParams::new();
        detector_params.insert("threshold".to_string(), 0.2.into());
        let detectors = HashMap::from([("fake_detector".to_string(), detector_params)]);
        let mut detection_streams = text_contents_detection_streams(
            ctx.clone(),
            HeaderMap::default(),
            detectors,
            0,
            input_rx,
        )
        .await?;

        // Spawn task to send inputs to input channel
        tokio::spawn(async move {
            for input in inputs {
                let _ = input_tx.send(Ok(input)).await;
            }
        });

        let mut fake_detector_stream = detection_streams.swap_remove(0);
        let mut results = Vec::with_capacity(1);
        while let Some(Ok((_input_id, _chunk, detections))) = fake_detector_stream.next().await {
            results.push(detections);
        }
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].len(), 1);

        Ok(())
    }
}
