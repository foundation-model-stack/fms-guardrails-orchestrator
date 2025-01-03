use axum_test::TestServer;
use common::{ensure_global_rustls_state, shared_state, ONCE};
use fms_guardrails_orchestr8::server::get_health_app;
use hyper::StatusCode;
use serde_json::Value;
use tracing::debug;
use tracing_test::traced_test;

mod common;

/// Checks if the health endpoint is working
/// NOTE: We do not currently mock client services yet, so this test is
/// superficially testing the client health endpoints on the orchestrator is accessible
/// and when the orchestrator is running (healthy) all the health endpoints return 200 OK.
/// This will happen even if the client services or their health endpoints are not found.
#[traced_test]
#[tokio::test]
async fn test_health() {
    ensure_global_rustls_state();
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
