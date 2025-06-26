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
use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use axum::{Router, extract::Request};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::{RootCertStore, ServerConfig, server::WebPkiClientVerifier};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::Service;
use tracing::{debug, error, info, warn};

/// Loads certificates and configures TLS.
pub fn configure_tls(
    tls_cert_path: Option<PathBuf>,
    tls_key_path: Option<PathBuf>,
    tls_client_ca_cert_path: Option<PathBuf>,
) -> Option<Arc<ServerConfig>> {
    if let (Some(cert_path), Some(key_path)) = (tls_cert_path, tls_key_path) {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let cert = load_certs(&cert_path);
        let key = load_private_key(&key_path);
        // Configure mTLS if client CA is provided
        let client_auth = if let Some(client_ca_cert_path) = tls_client_ca_cert_path {
            let client_certs = load_certs(&client_ca_cert_path);
            let mut client_auth_certs = RootCertStore::empty();
            for client_cert in client_certs {
                client_auth_certs
                    .add(client_cert.clone())
                    .unwrap_or_else(|e| panic!("error adding client cert {client_cert:?}: {e}"));
            }
            info!("mTLS enabled");
            WebPkiClientVerifier::builder(client_auth_certs.into())
                .build()
                .unwrap_or_else(|e| panic!("error building client verifier: {e}"))
        } else {
            info!("TLS enabled");
            WebPkiClientVerifier::no_client_auth()
        };
        let server_config = ServerConfig::builder()
            .with_client_cert_verifier(client_auth)
            .with_single_cert(cert, key)
            .expect("bad server certificate or key");
        Some(Arc::new(server_config))
    } else {
        info!("TLS not enabled");
        None
    }
}

/// Serve the service with the supplied listener, TLS config, and shutdown signal.
/// Based on https://github.com/tokio-rs/axum/blob/main/examples/low-level-rustls/src/main.rs
pub fn serve_with_tls<F>(
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

/// Load certificates from a file
fn load_certs(filename: &PathBuf) -> Vec<CertificateDer<'static>> {
    let cert_file = File::open(filename).expect("cannot open certificate file");
    let mut reader = BufReader::new(cert_file);
    rustls_pemfile::certs(&mut reader)
        .map(|result| result.unwrap())
        .collect()
}

/// Load private key from a file
fn load_private_key(filename: &PathBuf) -> PrivateKeyDer<'static> {
    let key_file = File::open(filename).expect("cannot open private key file");
    let mut reader = BufReader::new(key_file);
    loop {
        match rustls_pemfile::read_one(&mut reader).expect("cannot parse private key .pem file") {
            Some(rustls_pemfile::Item::Pkcs1Key(key)) => return key.into(),
            Some(rustls_pemfile::Item::Pkcs8Key(key)) => return key.into(),
            Some(rustls_pemfile::Item::Sec1Key(key)) => return key.into(),
            None => break,
            _ => {}
        }
    }
    panic!("no keys found in {filename:?} (encrypted keys not supported)");
}
