//! Client helpers
use std::sync::Arc;

use futures::{StreamExt, TryStreamExt};
use http::HeaderMap;
use tokio::sync::broadcast;
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tracing::{debug, instrument};

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
    orchestrator::{Context, Error, types::*},
    pb::caikit::runtime::chunkers::{
        BidiStreamingChunkerTokenizationTaskRequest, ChunkerTokenizationTaskRequest,
    },
};

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
    let client = ctx
        .clients
        .get_as::<GenerationClient>("generation")
        .unwrap();
    let stream = client
        .generate_stream(model_id.clone(), text, params, headers)
        .await
        .map_err(|error| Error::GenerateRequestFailed {
            id: model_id.clone(),
            error,
        })?
        .map_err(move |error| Error::GenerateRequestFailed {
            id: model_id.clone(),
            error,
        })
        .enumerate()
        .boxed();
    Ok(stream)
}
