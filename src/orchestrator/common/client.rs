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
//! Client helpers
use futures::{StreamExt, TryStreamExt};
use http::{HeaderMap, header::CONTENT_TYPE};
use tokio::sync::broadcast;
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tracing::{debug, instrument};

use crate::{
    clients::{
        DetectorClient, GenerationClient,
        chunker::ChunkerClient,
        detector::{
            ChatDetectionRequest, ContentAnalysisRequest, ContextDocsDetectionRequest, ContextType,
            GenerationDetectionRequest,
        },
        http::JSON_CONTENT_TYPE,
        openai::{self, OpenAiClient, TokenizeRequest},
    },
    models::{
        ClassifiedGeneratedTextResult as GenerateResponse, DetectorParams,
        GuardrailsTextGenerationParameters as GenerateParams,
    },
    orchestrator::{Error, types::*},
    pb::caikit::runtime::chunkers::{
        BidiStreamingChunkerTokenizationTaskRequest, ChunkerTokenizationTaskRequest,
    },
};

/// Sends request to chunker client.
#[instrument(skip_all, fields(chunker_id))]
pub async fn chunk(
    client: &ChunkerClient,
    chunker_id: ChunkerId,
    text: String,
) -> Result<Chunks, Error> {
    let request = ChunkerTokenizationTaskRequest { text };
    debug!(%chunker_id, ?request, "sending chunker request");
    let response = client
        .tokenization_task_predict(&chunker_id, request)
        .await
        .map_err(|error| Error::ChunkerRequestFailed {
            id: chunker_id.clone(),
            error,
        })?;
    debug!(%chunker_id, ?response, "received chunker response");
    Ok(response.into())
}

/// Sends chunk stream request to chunker client.
#[instrument(skip_all, fields(chunker_id))]
pub async fn chunk_stream(
    client: &ChunkerClient,
    chunker_id: ChunkerId,
    input_rx: broadcast::Receiver<Result<(usize, String), Error>>, // (message_index, text)
) -> Result<ChunkStream, Error> {
    let input_stream = BroadcastStream::new(input_rx)
        .map(|result| {
            let (index, text) = result.unwrap().unwrap();
            BidiStreamingChunkerTokenizationTaskRequest {
                text_stream: text,
                input_index_stream: index as i64,
            }
        })
        .boxed();
    debug!(%chunker_id, "sending chunk stream request");
    let output_stream = client
        .bidi_streaming_tokenization_task_predict(&chunker_id, input_stream)
        .await
        .map_err(|error| Error::ChunkerRequestFailed {
            id: chunker_id.clone(),
            error,
        })? // maps method call errors
        .map_ok(Into::into)
        .map_err(move |error| Error::ChunkerRequestFailed {
            id: chunker_id.clone(),
            error,
        }) // maps stream errors
        .boxed();
    Ok(output_stream)
}

/// Sends request to text contents detector client.
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_text_contents(
    client: &DetectorClient,
    headers: HeaderMap,
    detector_id: DetectorId,
    params: DetectorParams,
    chunks: Chunks,
    apply_chunk_offset: bool,
) -> Result<Detections, Error> {
    let detector_id = detector_id.clone();
    let contents = chunks
        .iter()
        .map(|chunk| chunk.text.clone())
        .collect::<Vec<_>>();
    if contents.is_empty() {
        return Ok(Detections::default());
    }
    let request = ContentAnalysisRequest::new(contents, params);
    debug!(%detector_id, ?request, "sending detector request");
    let response = client
        .text_contents(&detector_id, request, headers)
        .await
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(%detector_id, ?response, "received detector response");
    let detections = chunks
        .into_iter()
        .zip(response)
        .flat_map(|(chunk, detections)| {
            detections
                .into_iter()
                .map(|detection| {
                    let mut detection: Detection = detection.into();
                    detection.detector_id = Some(detector_id.clone());
                    if apply_chunk_offset {
                        let offset = chunk.start;
                        detection.start = detection.start.map(|start| start + offset);
                        detection.end = detection.end.map(|end| end + offset);
                    }
                    detection
                })
                .collect::<Vec<_>>()
        })
        .collect::<Detections>();
    Ok(detections)
}

/// Sends request to text generation detector client.
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_text_generation(
    client: &DetectorClient,
    headers: HeaderMap,
    detector_id: DetectorId,
    params: DetectorParams,
    prompt: String,
    generated_text: String,
) -> Result<Detections, Error> {
    let detector_id = detector_id.clone();
    let request = GenerationDetectionRequest::new(prompt, generated_text, params);
    debug!(%detector_id, ?request, "sending detector request");
    let response = client
        .text_generation(&detector_id, request, headers)
        .await
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(%detector_id, ?response, "received detector response");
    let detections = response
        .into_iter()
        .map(|detection| {
            let mut detection: Detection = detection.into();
            detection.detector_id = Some(detector_id.clone());
            detection
        })
        .collect::<Detections>();
    Ok(detections)
}

/// Sends request to text chat detector client.
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_text_chat(
    client: &DetectorClient,
    headers: HeaderMap,
    detector_id: DetectorId,
    params: DetectorParams,
    messages: Vec<openai::Message>,
    tools: Vec<openai::Tool>,
) -> Result<Detections, Error> {
    let detector_id = detector_id.clone();
    let request = ChatDetectionRequest::new(messages, tools, params);
    debug!(%detector_id, ?request, "sending detector request");
    let response = client
        .text_chat(&detector_id, request, headers)
        .await
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(%detector_id, ?response, "received detector response");
    let detections = response
        .into_iter()
        .map(|detection| {
            let mut detection: Detection = detection.into();
            detection.detector_id = Some(detector_id.clone());
            detection
        })
        .collect::<Detections>();
    Ok(detections)
}

/// Sends request to text context detector client.
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_text_context(
    client: &DetectorClient,
    headers: HeaderMap,
    detector_id: DetectorId,
    params: DetectorParams,
    content: String,
    context_type: ContextType,
    context: Vec<String>,
) -> Result<Detections, Error> {
    let detector_id = detector_id.clone();
    let request = ContextDocsDetectionRequest::new(content, context_type, context, params.clone());
    debug!(%detector_id, ?request, "sending detector request");
    let response = client
        .text_context_doc(&detector_id, request, headers)
        .await
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(%detector_id, ?response, "received detector response");
    let detections = response
        .into_iter()
        .map(|detection| {
            let mut detection: Detection = detection.into();
            detection.detector_id = Some(detector_id.clone());
            detection
        })
        .collect::<Detections>();
    Ok(detections)
}

/// Sends request to openai chat completions client.
#[instrument(skip_all, fields(model_id))]
pub async fn chat_completion(
    client: &OpenAiClient,
    mut headers: HeaderMap,
    request: openai::ChatCompletionsRequest,
) -> Result<openai::ChatCompletionsResponse, Error> {
    let model_id = request.model.clone();
    debug!(%model_id, ?request, "sending chat completions request");
    headers.append(CONTENT_TYPE, JSON_CONTENT_TYPE);
    let response = client
        .chat_completions(request, headers)
        .await
        .map_err(|error| Error::ChatCompletionRequestFailed {
            id: model_id.clone(),
            error,
        })?;
    debug!(%model_id, ?response, "received chat completions response");
    Ok(response)
}

/// Sends stream request to openai chat completions client.
#[instrument(skip_all, fields(model_id))]
pub async fn chat_completion_stream(
    client: &OpenAiClient,
    mut headers: HeaderMap,
    request: openai::ChatCompletionsRequest,
) -> Result<ChatCompletionStream, Error> {
    let model_id = request.model.clone();
    debug!(%model_id, ?request, "sending chat completions stream request");
    headers.append(CONTENT_TYPE, JSON_CONTENT_TYPE);
    let response = client
        .chat_completions(request, headers)
        .await
        .map_err(|error| Error::ChatCompletionRequestFailed {
            id: model_id.clone(),
            error,
        })?;
    let stream = match response {
        openai::ChatCompletionsResponse::Streaming(rx) => ReceiverStream::new(rx),
        openai::ChatCompletionsResponse::Unary(_) => unimplemented!(),
    }
    .enumerate()
    .boxed();
    Ok(stream)
}

/// Sends request to openai completions client.
#[instrument(skip_all, fields(model_id))]
pub async fn completion(
    client: &OpenAiClient,
    mut headers: HeaderMap,
    request: openai::CompletionsRequest,
) -> Result<openai::CompletionsResponse, Error> {
    let model_id = request.model.clone();
    debug!(%model_id, ?request, "sending completions request");
    headers.append(CONTENT_TYPE, JSON_CONTENT_TYPE);
    let response = client
        .completions(request, headers)
        .await
        .map_err(|error| Error::CompletionRequestFailed {
            id: model_id.clone(),
            error,
        })?;
    debug!(%model_id, ?response, "received completions response");
    Ok(response)
}

/// Sends stream request to openai completions client.
#[instrument(skip_all, fields(model_id))]
pub async fn completion_stream(
    client: &OpenAiClient,
    mut headers: HeaderMap,
    request: openai::CompletionsRequest,
) -> Result<CompletionStream, Error> {
    let model_id = request.model.clone();
    debug!(%model_id, ?request, "sending completions stream request");
    headers.append(CONTENT_TYPE, JSON_CONTENT_TYPE);
    let response = client
        .completions(request, headers)
        .await
        .map_err(|error| Error::CompletionRequestFailed {
            id: model_id.clone(),
            error,
        })?;
    let stream = match response {
        openai::CompletionsResponse::Streaming(rx) => ReceiverStream::new(rx),
        openai::CompletionsResponse::Unary(_) => unimplemented!(),
    }
    .enumerate()
    .boxed();
    Ok(stream)
}

/// Sends tokenize request to OpenAI client.
#[instrument(skip_all, fields(model_id))]
pub async fn tokenize_openai(
    client: &OpenAiClient,
    mut headers: HeaderMap,
    request: TokenizeRequest,
) -> Result<openai::TokenizeResponse, Error> {
    let model_id = request.model.clone();
    debug!(%model_id, ?request, "sending tokenize request");
    headers.append(CONTENT_TYPE, JSON_CONTENT_TYPE);
    let response = client.tokenize(request, headers).await.map_err(|error| {
        tracing::error!("Tokenize request failed: {error}");
        Error::TokenizeRequestFailed {
            id: model_id.clone(),
            error,
        }
    })?;
    debug!(%model_id, ?response, "received tokenize response");
    Ok(response)
}

/// Sends tokenize request to generation client.
#[instrument(skip_all, fields(model_id))]
pub async fn tokenize(
    client: &GenerationClient,
    headers: HeaderMap,
    model_id: String,
    text: String,
) -> Result<(u32, Vec<String>), Error> {
    // (token_count, tokens)
    debug!(%model_id, "sending tokenize request");
    let response = client
        .tokenize(model_id.clone(), text, headers)
        .await
        .map_err(|error| Error::TokenizeRequestFailed {
            id: model_id.clone(),
            error,
        })?;
    debug!(%model_id, ?response, "received tokenize response");
    Ok(response)
}

/// Sends generate request to generation client.
#[instrument(skip_all, fields(model_id))]
pub async fn generate(
    client: &GenerationClient,
    headers: HeaderMap,
    model_id: String,
    text: String,
    params: Option<GenerateParams>,
) -> Result<GenerateResponse, Error> {
    debug!(%model_id, "sending generate request");
    let response = client
        .generate(model_id.clone(), text, params, headers)
        .await
        .map_err(|error| Error::GenerateRequestFailed {
            id: model_id.clone(),
            error,
        })?;
    debug!(%model_id, ?response, "received generate response");
    Ok(response)
}

/// Sends generate stream request to generation client.
#[instrument(skip_all, fields(model_id))]
pub async fn generate_stream(
    client: &GenerationClient,
    headers: HeaderMap,
    model_id: String,
    text: String,
    params: Option<GenerateParams>,
) -> Result<GenerationStream, Error> {
    debug!(%model_id, "sending generate stream request");
    let stream = client
        .generate_stream(model_id.clone(), text, params, headers)
        .await
        .map_err(|error| Error::GenerateRequestFailed {
            id: model_id.clone(),
            error,
        })? // maps method call errors
        .map_err(move |error| Error::GenerateRequestFailed {
            id: model_id.clone(),
            error,
        }) // maps stream errors
        .enumerate()
        .boxed();
    Ok(stream)
}
