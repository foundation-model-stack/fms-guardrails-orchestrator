use std::sync::Arc;

use axum_test::TestServer;
use fms_guardrails_orchestr8::{
    config::OrchestratorConfig,
    orchestrator::Orchestrator,
    server::{get_health_app, ServerState},
};
use hyper::StatusCode;
use serde_json::Value;
use tokio::sync::OnceCell;
use tracing::debug;
use tracing_test::traced_test;

/// Async lazy initialization of shared state using tokio::sync::OnceCell
static ONCE: OnceCell<Arc<ServerState>> = OnceCell::const_new();

/// The actual async function that initializes the shared state if not already initialized
async fn shared_state() -> Arc<ServerState> {
    let config = OrchestratorConfig::load("tests/test.config.yaml")
        .await
        .unwrap();
    let orchestrator = Orchestrator::new(config, false).await.unwrap();
    Arc::new(ServerState::new(orchestrator))
}

/// Checks if the health endpoint is working
/// NOTE: We do not currently mock client services yet, so this test is
/// superficially testing the client health endpoints on the orchestrator is accessible
/// and when the orchestrator is running (healthy) all the health endpoints return 200 OK.
/// This will happen even if the client services or their health endpoints are not found.
#[traced_test]
#[tokio::test]
async fn test_health() {
    let shared_state = ONCE.get_or_init(shared_state).await.clone();
    let server = TestServer::new(get_health_app(shared_state)).unwrap();
    let response = server.get("/health").await;
    debug!("{:#?}", response);
    let body: Value = serde_json::from_str(response.text().as_str()).unwrap();
    debug!("{}", serde_json::to_string_pretty(&body).unwrap());
    response.assert_status(StatusCode::OK);
    let response = server.get("/info").await;
    println!("{:#?}", response);
    let body: Value = serde_json::from_str(response.text().as_str()).unwrap();
    println!("{}", serde_json::to_string_pretty(&body).unwrap());
    response.assert_status(StatusCode::OK);
}
