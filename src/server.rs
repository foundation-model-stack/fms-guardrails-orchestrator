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

use axum::{Router, extract::Request};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::{net::TcpListener, signal};
use tokio_rustls::TlsAcceptor;
use tower::Service;
use tower_http::trace::TraceLayer;
use tracing::{debug, error, info, warn};

use crate::orchestrator::Orchestrator;

mod tls;
use tls::configure_tls;
mod errors;
mod routes;
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
    let health_handle = run_health_server(health_addr, state.clone()).await;
    let guardrails_handle = run_guardrails_server(
        guardrails_addr,
        tls_cert_path,
        tls_key_path,
        tls_client_ca_cert_path,
        state.clone(),
    )
    .await;
    // Await server shutdown
    let _ = tokio::join!(health_handle, guardrails_handle);
    info!("shutdown complete");
    Ok(())
}

/// Configures and runs health server.
async fn run_health_server(
    addr: SocketAddr,
    state: Arc<ServerState>,
) -> tokio::task::JoinHandle<()> {
    info!("starting health server on {addr}");
    let app = routes::health_router(state);
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|_| panic!("starting health server failed: could not bind to {addr}"));
    let server =
        axum::serve(listener, app.into_make_service()).with_graceful_shutdown(shutdown_signal());
    tokio::task::spawn(async { server.await.expect("health server crashed!") })
}

/// Configures and runs guardrails server.
async fn run_guardrails_server(
    addr: SocketAddr,
    tls_cert_path: Option<PathBuf>,
    tls_key_path: Option<PathBuf>,
    tls_client_ca_cert_path: Option<PathBuf>,
    state: Arc<ServerState>,
) -> tokio::task::JoinHandle<()> {
    info!("starting guardrails server on {addr}");
    let router = routes::guardrails_router(state);
    let app = router.layer(
        TraceLayer::new_for_http()
            .make_span_with(crate::utils::trace::incoming_request_span)
            .on_request(crate::utils::trace::on_incoming_request)
            .on_response(crate::utils::trace::on_outgoing_response)
            .on_eos(crate::utils::trace::on_outgoing_eos),
    );
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|_| panic!("starting guardrails server failed: could not bind to {addr}"));
    let tls_config = configure_tls(tls_cert_path, tls_key_path, tls_client_ca_cert_path);
    let shutdown_signal = shutdown_signal();
    if let Some(tls_config) = tls_config {
        serve_with_tls(app, listener, tls_config, shutdown_signal)
    } else {
        let server =
            axum::serve(listener, app.into_make_service()).with_graceful_shutdown(shutdown_signal);
        tokio::task::spawn(async { server.await.expect("guardrails server crashed!") })
    }
}

/// Serve the service with the supplied listener, TLS config, and shutdown signal.
/// Based on https://github.com/tokio-rs/axum/blob/main/examples/low-level-rustls/src/main.rs
fn serve_with_tls<F>(
    app: Router,
    listener: TcpListener,
    tls_config: Arc<rustls::ServerConfig>,
    shutdown_signal: F,
) -> tokio::task::JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let tls_acceptor = TlsAcceptor::from(tls_config);
    tokio::spawn(async move {
        let graceful = hyper_util::server::graceful::GracefulShutdown::new();
        let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
        let mut signal = std::pin::pin!(shutdown_signal);
        loop {
            let tower_service = app.clone();
            let tls_acceptor = tls_acceptor.clone();
            // Wait for new tcp connection
            let (cnx, addr) = tokio::select! {
                res = listener.accept() => {
                    match res {
                        Ok(res) => res,
                        Err(err) => {
                            error!("error accepting tcp connection: {err}");
                            continue;
                        }
                    }
                }
                _ = &mut signal => {
                    debug!("graceful shutdown signal received");
                    break;
                }
            };
            // Wait for tls handshake
            let stream = tokio::select! {
                res = tls_acceptor.accept(cnx) => {
                    match res {
                        Ok(stream) => stream,
                        Err(err) => {
                            error!("error accepting connection on handshake: {err}");
                            continue;
                        }
                    }
                }
                _ = &mut signal => {
                    debug!("graceful shutdown signal received");
                    break;
                }
            };
            // `TokioIo` converts between Hyper's own `AsyncRead` and `AsyncWrite` traits
            let stream = TokioIo::new(stream);
            let hyper_service = hyper::service::service_fn(move |request: Request<Incoming>| {
                // Clone necessary since hyper's `Service` uses `&self` whereas
                // tower's `Service` requires `&mut self`
                tower_service.clone().call(request)
            });
            let conn = builder.serve_connection_with_upgrades(stream, hyper_service);
            let fut = graceful.watch(conn.into_owned());
            tokio::spawn(async move {
                if let Err(err) = fut.await {
                    warn!("error serving connection from {}: {}", addr, err);
                }
            });
        }
        tokio::select! {
            () = graceful.shutdown() => {
                debug!("graceful shutdown completed");
            },
            () = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                debug!("graceful shutdown timed out, aborting...");
            }
        }
    })
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
