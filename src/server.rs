use std::{net::SocketAddr, time::Duration};

use axum::{
    extract::Extension,
    http::{HeaderMap, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures::stream::Any;
use serde_json::{json, Value};
use tokio::{fs::read, signal, time::sleep};
use tracing::info;


const API_PREFIX: &'static str = r#"/api/v1/task"#;


// Server shared state
#[derive(Clone)]
pub(crate) struct ServerState {
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    rest_addr: SocketAddr,
    // tls_key_pair: Option<(String, String)>,
    // detector_map: DetectorMap,
) {

    // TODO: Configure TLS if requested

    // Build and await on the HTTP server
    let app = Router::new()
        .route("/health", get(health))
        .route(&format!("{}/classification-with-text-generation", API_PREFIX), post(classification_with_generation));

    let server = axum::Server::bind(&rest_addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal());

    info!("HTTP server started on port {}", rest_addr.port());
    server.await.unwrap();
    info!("HTTP server shutdown complete");

}

async fn health() -> Result<(), ()> {
    // TODO: determine how to detect if orchestrator is healthy or not
    Ok(())
}

// TODO: Change with real object
struct inner_response {
    sample: bool
}
struct sample_response {
    response: inner_response
}

async fn classification_with_generation(Json(payload): Json<Value>) ->  (StatusCode, Json<Value>) {
    // TODO: determine how to detect if orchestrator is healthy or not
    let response = json!({"response": {"sample": true}});
    (StatusCode::OK, Json(response))
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