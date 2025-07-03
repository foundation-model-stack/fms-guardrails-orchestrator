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
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use axum_extra::{extract::WithRejection, json_lines::JsonLines};
use futures::{
    Stream, StreamExt,
    stream::{self, BoxStream},
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;

use super::{Error, ServerState};
use crate::{
    clients::openai::{
        ChatCompletionsRequest, ChatCompletionsResponse, CompletionsRequest, CompletionsResponse,
    },
    models::{self, InfoParams, InfoResponse, StreamingContentDetectionRequest},
    orchestrator::{
        self,
        handlers::{
            chat_completions_detection::ChatCompletionsDetectionTask,
            completions_detection::CompletionsDetectionTask, *,
        },
    },
    utils::{self, trace::current_trace_id},
};

const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
const PACKAGE_NAME: &str = env!("CARGO_PKG_NAME");

/// Creates health router.
pub fn health_router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/info", get(info))
        .with_state(state)
}

/// Creates guardrails router.
pub fn guardrails_router(state: Arc<ServerState>) -> Router {
    let mut router = Router::new()
        // v1 routes
        .route(
            "/api/v1/task/classification-with-text-generation",
            post(classification_with_gen),
        )
        .route(
            "/api/v1/task/server-streaming-classification-with-text-generation",
            post(stream_classification_with_gen),
        )
        // v2 routes
        .route(
            "/api/v2/text/detection/stream-content",
            post(stream_content_detection),
        )
        .route(
            "/api/v2/text/generation-detection",
            post(generation_with_detection),
        )
        .route("/api/v2/text/detection/content", post(detection_content))
        .route("/api/v2/text/detection/chat", post(detect_chat))
        .route(
            "/api/v2/text/detection/context",
            post(detect_context_documents),
        )
        .route("/api/v2/text/detection/generated", post(detect_generated));
    if state.orchestrator.config().openai.is_some() {
        info!("Enabling chat completions detection endpoint");
        router = router.route(
            "/api/v2/chat/completions-detection",
            post(chat_completions_detection),
        );
    }
    if state.orchestrator.config().completions.is_some() {
        info!("Enabling completions detection endpoint");
        router = router.route("/api/v2/chat/completions", post(completions_detection));
    }
    router.with_state(state)
}

async fn health() -> Result<impl IntoResponse, ()> {
    // NOTE: we are only adding the package information in the `health` endpoint to have this endpoint
    // provide a non empty 200 response. If we need to add more information regarding dependencies version
    // or such things, then we will add another `/info` endpoint accordingly. And those info
    // should not be added in `health` endpoint`
    let info_object = HashMap::from([(PACKAGE_NAME, PACKAGE_VERSION)]);
    Ok(Json(info_object).into_response())
}

async fn info(
    State(state): State<Arc<ServerState>>,
    Query(params): Query<InfoParams>,
) -> Result<Json<InfoResponse>, Error> {
    let services = state.orchestrator.client_health(params.probe).await;
    Ok(Json(InfoResponse { services }))
}

async fn classification_with_gen(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    WithRejection(Json(request), _): WithRejection<Json<models::GuardrailsHttpRequest>, Error>,
) -> Result<impl IntoResponse, Error> {
    let trace_id = current_trace_id();
    request.validate()?;
    let headers = filter_headers(&state.orchestrator.config().passthrough_headers, headers);
    let task = ClassificationWithGenTask::new(trace_id, request, headers);
    match state.orchestrator.handle(task).await {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn generation_with_detection(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    WithRejection(Json(request), _): WithRejection<
        Json<models::GenerationWithDetectionHttpRequest>,
        Error,
    >,
) -> Result<impl IntoResponse, Error> {
    let trace_id = current_trace_id();
    request.validate()?;
    let headers = filter_headers(&state.orchestrator.config().passthrough_headers, headers);
    let task = GenerationWithDetectionTask::new(trace_id, request, headers);
    match state.orchestrator.handle(task).await {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn stream_classification_with_gen(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    WithRejection(Json(request), _): WithRejection<Json<models::GuardrailsHttpRequest>, Error>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let trace_id = current_trace_id();
    if let Err(error) = request.validate() {
        // Request validation failed, return stream with single error SSE event
        let error: Error = error.into();
        return Sse::new(
            stream::iter([Ok(Event::default()
                .event("error")
                .json_data(error)
                .unwrap())])
            .boxed(),
        );
    }
    let headers = filter_headers(&state.orchestrator.config().passthrough_headers, headers);
    let task = StreamingClassificationWithGenTask::new(trace_id, request, headers);
    let response_stream = state.orchestrator.handle(task).await.unwrap();
    // Convert response stream to a stream of SSE events
    let event_stream = response_stream
        .map(|message| match message {
            Ok(response) => Ok(Event::default()
                //.event("message") NOTE: per spec, should not be included for data-only message events
                .json_data(response)
                .unwrap()),
            Err(error) => {
                let error: Error = error.into();
                Ok(Event::default().event("error").json_data(error).unwrap())
            }
        })
        .boxed();
    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

async fn stream_content_detection(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    json_lines: JsonLines<StreamingContentDetectionRequest>,
) -> Result<impl IntoResponse, Error> {
    let trace_id = current_trace_id();
    // Validate the content-type from the header and ensure it is application/x-ndjson
    // If it's not, return a UnsupportedContentType error with the appropriate message
    let content_type = headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    match content_type {
        Some(content_type) if content_type.starts_with("application/x-ndjson") => (),
        _ => {
            return Err(Error {
                code: http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                details: "expected application/x-ndjson".into(),
            });
        }
    };
    let headers = filter_headers(&state.orchestrator.config().passthrough_headers, headers);

    // Create input stream
    let input_stream = json_lines
        .map(|result| match result {
            Ok(message) => {
                message.validate()?;
                Ok(message)
            }
            Err(error) => Err(orchestrator::errors::Error::Validation(error.to_string())),
        })
        .enumerate()
        .boxed();

    // Create task and submit to handler
    let task = StreamingContentDetectionTask::new(trace_id, headers, input_stream);
    let mut response_stream = state.orchestrator.handle(task).await?;

    // Create output stream
    // This stream returns ND-JSON formatted messages to the client
    // StreamingContentDetectionResponse / server::Error
    let (output_tx, output_rx) = mpsc::channel::<Result<String, Infallible>>(128);
    let output_stream = ReceiverStream::new(output_rx);

    // Spawn task to consume response stream (typed) and send to output stream (json)
    tokio::spawn(async move {
        while let Some(result) = response_stream.next().await {
            match result {
                Ok(msg) => {
                    let msg = utils::json::to_nd_string(&msg).unwrap();
                    let _ = output_tx.send(Ok(msg)).await;
                }
                Err(error) => {
                    // Convert orchestrator::Error to server::Error
                    let error: Error = error.into();
                    let error_msg = utils::json::to_nd_string(&error).unwrap();
                    let _ = output_tx.send(Ok(error_msg)).await;
                }
            }
        }
    });

    Ok(Response::new(axum::body::Body::from_stream(output_stream)))
}

async fn detection_content(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    WithRejection(Json(request), _): WithRejection<
        Json<models::TextContentDetectionHttpRequest>,
        Error,
    >,
) -> Result<impl IntoResponse, Error> {
    let trace_id = current_trace_id();
    request.validate()?;
    let headers = filter_headers(&state.orchestrator.config().passthrough_headers, headers);
    let task = TextContentDetectionTask::new(trace_id, request, headers);
    match state.orchestrator.handle(task).await {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn detect_context_documents(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    WithRejection(Json(request), _): WithRejection<Json<models::ContextDocsHttpRequest>, Error>,
) -> Result<impl IntoResponse, Error> {
    let trace_id = current_trace_id();
    request.validate()?;
    let headers = filter_headers(&state.orchestrator.config().passthrough_headers, headers);
    let task = ContextDocsDetectionTask::new(trace_id, request, headers);
    match state.orchestrator.handle(task).await {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn detect_chat(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    WithRejection(Json(request), _): WithRejection<Json<models::ChatDetectionHttpRequest>, Error>,
) -> Result<impl IntoResponse, Error> {
    let trace_id = current_trace_id();
    request.validate_for_text()?;
    let headers = filter_headers(&state.orchestrator.config().passthrough_headers, headers);
    let task = ChatDetectionTask::new(trace_id, request, headers);
    match state.orchestrator.handle(task).await {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn detect_generated(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    WithRejection(Json(request), _): WithRejection<
        Json<models::DetectionOnGeneratedHttpRequest>,
        Error,
    >,
) -> Result<impl IntoResponse, Error> {
    let trace_id = current_trace_id();
    request.validate()?;
    let headers = filter_headers(&state.orchestrator.config().passthrough_headers, headers);
    let task = DetectionOnGenerationTask::new(trace_id, request, headers);
    match state.orchestrator.handle(task).await {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn chat_completions_detection(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    WithRejection(Json(request), _): WithRejection<Json<ChatCompletionsRequest>, Error>,
) -> Result<impl IntoResponse, Error> {
    use ChatCompletionsResponse::*;
    let trace_id = current_trace_id();
    request.validate()?;
    let headers = filter_headers(&state.orchestrator.config().passthrough_headers, headers);
    let task = ChatCompletionsDetectionTask::new(trace_id, request, headers);
    match state.orchestrator.handle(task).await {
        Ok(response) => match response {
            Unary(response) => Ok(Json(response).into_response()),
            Streaming(response_rx) => {
                let response_stream = ReceiverStream::new(response_rx);
                // Convert response stream to a stream of SSE events
                let event_stream: BoxStream<Result<Event, Infallible>> = response_stream
                    .map(|message| match message {
                        Ok(Some(chunk)) => Ok(Event::default().json_data(chunk).unwrap()),
                        Ok(None) => {
                            // The stream completed, send [DONE] message
                            Ok(Event::default().data("[DONE]"))
                        }
                        Err(error) => {
                            let error: Error = error.into();
                            Ok(Event::default().event("error").json_data(error).unwrap())
                        }
                    })
                    .boxed();
                let sse = Sse::new(event_stream).keep_alive(KeepAlive::default());
                Ok(sse.into_response())
            }
        },
        Err(error) => Err(error.into()),
    }
}

async fn completions_detection(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    WithRejection(Json(request), _): WithRejection<Json<CompletionsRequest>, Error>,
) -> Result<impl IntoResponse, Error> {
    use CompletionsResponse::*;
    let trace_id = current_trace_id();
    request.validate()?;
    let headers = filter_headers(&state.orchestrator.config().passthrough_headers, headers);
    let task = CompletionsDetectionTask::new(trace_id, request, headers);
    match state.orchestrator.handle(task).await {
        Ok(response) => match response {
            Unary(response) => Ok(Json(response).into_response()),
            Streaming(response_rx) => {
                let response_stream = ReceiverStream::new(response_rx);
                // Convert response stream to a stream of SSE events
                let event_stream: BoxStream<Result<Event, Infallible>> = response_stream
                    .map(|message| match message {
                        Ok(Some(chunk)) => Ok(Event::default().json_data(chunk).unwrap()),
                        Ok(None) => {
                            // The stream completed, send [DONE] message
                            Ok(Event::default().data("[DONE]"))
                        }
                        Err(error) => {
                            let error: Error = error.into();
                            Ok(Event::default().event("error").json_data(error).unwrap())
                        }
                    })
                    .boxed();
                let sse = Sse::new(event_stream).keep_alive(KeepAlive::default());
                Ok(sse.into_response())
            }
        },
        Err(error) => Err(error.into()),
    }
}

/// Filters a [`HeaderMap`] with a set of header names, returning a new [`HeaderMap`].
pub fn filter_headers(passthrough_headers: &HashSet<String>, headers: HeaderMap) -> HeaderMap {
    headers
        .iter()
        .filter(|(name, _)| passthrough_headers.contains(&name.as_str().to_lowercase()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
