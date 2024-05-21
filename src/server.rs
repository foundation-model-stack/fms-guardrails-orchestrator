use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use tokio::{net::TcpListener, signal};
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    config::OrchestratorConfig,
    models,
    orchestrator::{
        self, ClassificationWithGenTask, Orchestrator, StreamingClassificationWithGenTask,
    },
};

const API_PREFIX: &str = r#"/api/v1/task"#;

/// Server shared state
pub struct ServerState {
    orchestrator: Orchestrator,
}

/// Run the orchestrator server
pub async fn run(
    http_addr: SocketAddr,
    _tls_cert_path: Option<PathBuf>,
    _tls_key_path: Option<PathBuf>,
    _tls_client_ca_cert_path: Option<PathBuf>,
    config_path: PathBuf,
) -> Result<(), Error> {
    let config = OrchestratorConfig::load(config_path).await;
    let orchestrator = Orchestrator::new(config).await?;
    let shared_state = Arc::new(ServerState { orchestrator });

    // TODO: configure server TLS

    // Build and await on the HTTP server
    let app = Router::new()
        .route("/health", get(health))
        .route(
            &format!("{}/classification-with-text-generation", API_PREFIX),
            post(classification_with_gen),
        )
        .route(
            &format!(
                "{}/server-streaming-classification-with-text-generation",
                API_PREFIX
            ),
            post(stream_classification_with_gen),
        )
        .with_state(shared_state);

    let listener = TcpListener::bind(&http_addr)
        .await
        .unwrap_or_else(|_| panic!("failed to bind to {http_addr}"));
    let server =
        axum::serve(listener, app.into_make_service()).with_graceful_shutdown(shutdown_signal());

    info!("HTTP server started on port {}", http_addr.port());
    server.await.expect("HTTP server crashed!");
    info!("HTTP server shutdown complete");
    Ok(())
}

async fn health() -> Result<(), ()> {
    // TODO: determine how to detect if orchestrator is healthy or not
    Ok(())
}

async fn classification_with_gen(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<models::GuardrailsHttpRequest>,
) -> Result<impl IntoResponse, Error> {
    let request_id = Uuid::new_v4();
    let task = ClassificationWithGenTask::new(request_id, request);
    match state
        .orchestrator
        .handle_classification_with_gen(task)
        .await
    {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn stream_classification_with_gen(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<models::GuardrailsHttpRequest>,
) -> Result<impl IntoResponse, Error> {
    let request_id = Uuid::new_v4();
    let task = StreamingClassificationWithGenTask::new(request_id, request);
    let response_stream = state
        .orchestrator
        .handle_streaming_classification_with_gen(task)
        .await
        .map(|response| Event::default().json_data(response));
    let sse = Sse::new(response_stream).keep_alive(KeepAlive::default());
    Ok(sse.into_response())
}

/// Shutdown signal handler
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("signal received, starting graceful shutdown");
}

/// High-level errors to return to clients.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    NotFound(String),
    #[error("unexpected error occured while processing request")]
    Unexpected,
}

impl From<orchestrator::Error> for Error {
    fn from(error: orchestrator::Error) -> Self {
        use orchestrator::Error::*;
        match error {
            DetectorNotFound { .. } => Self::NotFound(error.to_string()),
            DetectorRequestFailed { error, .. }
            | ChunkerRequestFailed { error, .. }
            | GenerateRequestFailed { error, .. }
            | TokenizeRequestFailed { error, .. } => match error.status_code() {
                StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                    Self::Validation(error.to_string())
                }
                StatusCode::NOT_FOUND => Self::NotFound(error.to_string()),
                _ => Self::Unexpected,
            },
            _ => Self::Unexpected,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        use Error::*;
        let (code, message) = match self {
            Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Unexpected => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        let error = serde_json::json!({
            "code": code.as_u16(),
            "details": message,
        });
        (code, Json(error)).into_response()
    }
}
