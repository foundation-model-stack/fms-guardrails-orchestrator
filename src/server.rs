use std::{fs::File, io::BufReader, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

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
use axum_server::tls_rustls::RustlsConfig;
use futures::StreamExt;
use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use rustls::ServerConfig;
use tokio::{net::TcpListener, signal};
use tracing::{error, info};
use uuid::Uuid;
use webpki::types::{CertificateDer, PrivateKeyDer};

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
    health_http_addr: SocketAddr,
    tls_cert_path: Option<PathBuf>,
    tls_key_path: Option<PathBuf>,
    tls_client_ca_cert_path: Option<PathBuf>,
    config_path: PathBuf,
) -> Result<(), Error> {
    let config = OrchestratorConfig::load(config_path).await;
    let orchestrator = Orchestrator::new(config).await?;
    let shared_state = Arc::new(ServerState { orchestrator });

    // Separate HTTP health server without TLS for probes
    let health_app: Router = Router::new().route("/health", get(health));
    let listener = TcpListener::bind(&health_http_addr)
        .await
        .unwrap_or_else(|_| panic!("failed to bind to {health_http_addr}"));
    let health_server = axum::serve(listener, health_app.into_make_service())
        .with_graceful_shutdown(shutdown_signal(None));
    let health_handle =
        tokio::task::spawn(async { health_server.await.expect("HTTP health server crashed!") });
    info!(
        "HTTP health server started on port {}",
        health_http_addr.port()
    );

    // Main HTTP server
    let mut arc_server_config: Option<Arc<ServerConfig>> = None;
    // Configure TLS if requested
    if let (Some(cert_path), Some(key_path)) = (tls_cert_path, tls_key_path) {
        info!("Configuring Server TLS for incoming connections");
        let server_cert = load_certs(&cert_path);
        let key = load_private_key(&key_path);

        let client_auth = if tls_client_ca_cert_path.is_some() {
            info!("Configuring TLS trust certificate (mTLS) for incoming connections");
            let client_certs = load_certs(tls_client_ca_cert_path.as_ref().unwrap());
            let mut client_auth_certs = RootCertStore::empty();
            for client_cert in client_certs {
                // Should be only one
                client_auth_certs.add(client_cert).unwrap();
            }
            WebPkiClientVerifier::builder(client_auth_certs.into())
                .build()
                .unwrap()
        } else {
            WebPkiClientVerifier::no_client_auth()
        };
        let server_config = ServerConfig::builder()
            .with_client_cert_verifier(client_auth)
            .with_single_cert(server_cert, key)
            .expect("bad server certificate or key");
        arc_server_config = Some(Arc::new(server_config));
    } else {
        info!("HTTP server not configured with TLS")
    }

    // Handle for shutdown signal ability
    let handle = axum_server::Handle::new();
    let shutdown_future = shutdown_signal(Some(handle.clone()));
    // Separate task for shutdown - is this ideal
    tokio::spawn(shutdown_future);

    let app = Router::new()
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

    // Launch each app as a separate task
    let handle = if arc_server_config.is_some() {
        // TODO: incompatible ServerConfig :(
        let tls_config = RustlsConfig::from_config(arc_server_config.unwrap());
        let https_server = axum_server::bind_rustls(http_addr, tls_config)
            .handle(handle)
            .serve(app.into_make_service());
        tokio::task::spawn(async { https_server.await.expect("HTTPS server crashed!") })
    } else {
        let http_server = axum_server::bind(http_addr)
            .handle(handle)
            .serve(app.into_make_service());
        tokio::task::spawn(async { http_server.await.expect("HTTP server crashed!") })
    };
    info!("HTTP server started on port {}", http_addr.port());

    let (health_res, res) = tokio::join!(health_handle, handle);
    health_res.unwrap();
    res.unwrap();
    info!("Shutdown complete for HTTP servers");
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
    // Upfront request validation
    request.validate()?;
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
async fn shutdown_signal(handle: Option<axum_server::Handle>) {
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
    if let Some(axum_handle) = handle {
        // TODO: Make configurable
        axum_handle.graceful_shutdown(Some(Duration::from_secs(10)))
    }
}

// Ref. https://github.com/rustls/rustls/blob/main/examples/src/bin/tlsserver-mio.rs
fn load_certs(filename: &PathBuf) -> Vec<CertificateDer<'static>> {
    let certfile = File::open(filename).expect("cannot open certificate file");
    let mut reader = BufReader::new(certfile);
    rustls_pemfile::certs(&mut reader)
        .map(|result| result.unwrap())
        .collect()
}

fn load_private_key(filename: &PathBuf) -> PrivateKeyDer<'static> {
    let keyfile = File::open(filename).expect("cannot open private key file");
    let mut reader = BufReader::new(keyfile);

    loop {
        match rustls_pemfile::read_one(&mut reader).expect("cannot parse private key .pem file") {
            Some(rustls_pemfile::Item::Pkcs1Key(key)) => return key.into(),
            Some(rustls_pemfile::Item::Pkcs8Key(key)) => return key.into(),
            Some(rustls_pemfile::Item::Sec1Key(key)) => return key.into(),
            None => break,
            _ => {}
        }
    }

    panic!(
        "no keys found in {:?} (encrypted keys not supported)",
        filename
    );
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

impl From<models::ValidationError> for Error {
    fn from(value: models::ValidationError) -> Self {
        Self::Validation(value.to_string())
    }
}
