use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use tokio::{net::TcpListener, signal};
use tracing::info;

use crate::{
    clients,
    config::OrchestratorConfig,
    models,
    orchestrator::{ClassificationWithGenTask, Orchestrator, StreamingClassificationWithGenTask},
    Error,
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

    let listener = TcpListener::bind(&http_addr).await?;
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
) -> Result<impl IntoResponse, (StatusCode, Json<String>)> {
    let task = ClassificationWithGenTask::new(request);
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
) -> Result<impl IntoResponse, (StatusCode, Json<String>)> {
    let task = StreamingClassificationWithGenTask::new(request);
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

// TODO: create better errors and properly convert
impl From<Error> for (StatusCode, Json<String>) {
    fn from(value: Error) -> Self {
        use Error::*;
        match value {
            ClientError(error) => match error {
                clients::Error::ModelNotFound(message) => {
                    (StatusCode::UNPROCESSABLE_ENTITY, Json(message))
                }
                clients::Error::ReqwestError(error) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, Json(error.to_string()))
                }
                clients::Error::TonicError(error) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, Json(error.to_string()))
                }
                clients::Error::IoError(_) => todo!(),
            },
            IoError(_) => todo!(),
            YamlError(_) => todo!(),
        }
    }
}
