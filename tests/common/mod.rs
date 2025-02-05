use std::sync::Arc;

use fms_guardrails_orchestr8::{
    config::OrchestratorConfig, orchestrator::Orchestrator, server::ServerState,
};
use rustls::crypto::ring;
use tokio::sync::OnceCell;

/// Async lazy initialization of shared state using tokio::sync::OnceCell
pub static ONCE: OnceCell<Arc<ServerState>> = OnceCell::const_new();

pub const CONFIG_FILE_PATH: &str = "tests/test.config.yaml";

/// The actual async function that initializes the shared state if not already initialized
pub async fn shared_state() -> Arc<ServerState> {
    let config = OrchestratorConfig::load("tests/test.config.yaml")
        .await
        .unwrap();
    let orchestrator = Orchestrator::new(config, false).await.unwrap();
    Arc::new(ServerState::new(orchestrator))
}

pub fn ensure_global_rustls_state() {
    let _ = ring::default_provider().install_default();
}
