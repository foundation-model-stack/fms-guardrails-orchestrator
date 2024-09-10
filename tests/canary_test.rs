use axum_test::TestServer;
use fms_guardrails_orchestr8::server::get_health_app;
use hyper::StatusCode;

/// Checks if the health endpoint is working
#[tokio::test]
async fn test_health() {
    let server = TestServer::new(get_health_app()).unwrap();
    let response = server.get("/health").await;
    response.assert_status(StatusCode::OK);
}
