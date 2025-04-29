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
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use tokio::{net::TcpListener, signal};
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::orchestrator::Orchestrator;

mod errors;
mod routes;
mod tls;
use tls::{configure_tls, serve_with_tls};
mod utils;
pub use errors::Error;

/// Configures and runs orchestrator servers.
pub async fn run(
    guardrails_addr: SocketAddr,
    health_addr: SocketAddr,
    tls_cert_path: Option<PathBuf>,
    tls_key_path: Option<PathBuf>,
    tls_client_ca_cert_path: Option<PathBuf>,
    orchestrator: Orchestrator,
) -> Result<(), Error> {
    let state = Arc::new(ServerState::new(orchestrator));
    let health_handle = run_health_server(health_addr, state.clone()).await?;
    let guardrails_handle = run_guardrails_server(
        guardrails_addr,
        tls_cert_path,
        tls_key_path,
        tls_client_ca_cert_path,
        state,
    )
    .await?;
    // Await server shutdown
    let _ = tokio::join!(health_handle, guardrails_handle);
    info!("shutdown complete");
    Ok(())
}

/// Configures and runs health server.
async fn run_health_server(
    addr: SocketAddr,
    state: Arc<ServerState>,
) -> Result<tokio::task::JoinHandle<()>, Error> {
    info!("starting health server on {addr}");
    let app = routes::health_router(state);
    let listener = TcpListener::bind(&addr).await?;
    let server =
        axum::serve(listener, app.into_make_service()).with_graceful_shutdown(shutdown_signal());
    Ok(tokio::task::spawn(async {
        server.await.expect("health server crashed!")
    }))
}

/// Configures and runs guardrails server.
async fn run_guardrails_server(
    addr: SocketAddr,
    tls_cert_path: Option<PathBuf>,
    tls_key_path: Option<PathBuf>,
    tls_client_ca_cert_path: Option<PathBuf>,
    state: Arc<ServerState>,
) -> Result<tokio::task::JoinHandle<()>, Error> {
    info!("starting guardrails server on {addr}");
    let router = routes::guardrails_router(state);
    let app = router.layer(
        TraceLayer::new_for_http()
            .make_span_with(crate::utils::trace::incoming_request_span)
            .on_request(crate::utils::trace::on_incoming_request)
            .on_response(crate::utils::trace::on_outgoing_response)
            .on_eos(crate::utils::trace::on_outgoing_eos),
    );
    let listener = TcpListener::bind(&addr).await?;
    let tls_config = configure_tls(tls_cert_path, tls_key_path, tls_client_ca_cert_path);
    let shutdown_signal = shutdown_signal();
    if let Some(tls_config) = tls_config {
        Ok(serve_with_tls(app, listener, tls_config, shutdown_signal))
    } else {
        let server =
            axum::serve(listener, app.into_make_service()).with_graceful_shutdown(shutdown_signal);
        Ok(tokio::task::spawn(async {
            server.await.expect("guardrails server crashed!")
        }))
    }
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

/// Server shared state
pub struct ServerState {
    orchestrator: Orchestrator,
}

impl ServerState {
    pub fn new(orchestrator: Orchestrator) -> Self {
        Self { orchestrator }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_run_bind_failure() -> Result<(), Error> {
        let guardrails_addr: SocketAddr = "0.0.0.0:50101".parse().unwrap();
        let health_addr: SocketAddr = "0.0.0.0:50103".parse().unwrap();
        let _listener = TcpListener::bind(&guardrails_addr).await?;
        let result = run(
            guardrails_addr,
            health_addr,
            None,
            None,
            None,
            Orchestrator::default(),
        )
        .await;
        assert!(result.is_err_and(|error| matches!(error, Error::IoError(_))
            && error.to_string().starts_with("Address already in use")));
        Ok(())
    }
}
