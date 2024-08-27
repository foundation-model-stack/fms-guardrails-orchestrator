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

use std::{
    collections::HashMap, convert::Infallible, error::Error as _, fs::File, io::BufReader,
    net::SocketAddr, path::PathBuf, sync::Arc,
};

use axum::{
    extract::{rejection::JsonRejection, Request, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use axum_extra::extract::WithRejection;
use futures::{stream, Stream, StreamExt};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::{server::WebPkiClientVerifier, RootCertStore, ServerConfig};
use tokio::{net::TcpListener, signal};
use tokio_rustls::TlsAcceptor;
use tower_service::Service;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use webpki::types::{CertificateDer, PrivateKeyDer};

use crate::{
    models::{self},
    orchestrator::{
        self, ClassificationWithGenTask, ContextDocsDetectionTask, DetectionOnGenerationTask,
        GenerationWithDetectionTask, Orchestrator, StreamingClassificationWithGenTask,
        TextContentDetectionTask,
    },
};

const API_PREFIX: &str = r#"/api/v1/task"#;
// New orchestrator API
const TEXT_API_PREFIX: &str = r#"/api/v2/text"#;

const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
const PACKAGE_NAME: &str = env!("CARGO_PKG_NAME");

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
    orchestrator: Orchestrator,
) -> Result<(), Error> {
    // Overall, the server setup and run does a couple of steps:
    // (1) Sets up a HTTP server (without TLS) for the health endpoint
    // (2) Sets up a HTTP(s) server for the main guardrails endpoints
    //     (2a) Configures TLS or mTLS depending on certs/key provided
    //     (2b) Adds server routes
    //     (2c) Generate the server task based on whether or not TLS is configured
    // (3) Launch each server as a separate task
    // NOTE: axum::serve is used for servers without TLS since it is designed to be
    // simple and not allow for much configuration. To allow for TLS configuration
    // with rustls, the hyper and tower crates [what axum is built on] had to
    // be used directly

    let shared_state = Arc::new(ServerState { orchestrator });

    // (1) Separate HTTP health server without TLS for probes
    let health_app: Router = Router::new().route("/health", get(health));
    let health_listener = TcpListener::bind(&health_http_addr)
        .await
        .unwrap_or_else(|_| panic!("failed to bind to {health_http_addr}"));
    let health_server = axum::serve(health_listener, health_app.into_make_service())
        .with_graceful_shutdown(shutdown_signal());
    let health_handle =
        tokio::task::spawn(async { health_server.await.expect("HTTP health server crashed!") });
    info!(
        "HTTP health server started on port {}",
        health_http_addr.port()
    );

    // (2) Main guardrails server
    // (2a) Configure TLS if requested
    let mut arc_server_config: Option<Arc<ServerConfig>> = None;
    if let (Some(cert_path), Some(key_path)) = (tls_cert_path, tls_key_path) {
        info!("Configuring Server TLS for incoming connections");
        let server_cert = load_certs(&cert_path);
        let key = load_private_key(&key_path);

        // A process wide default crypto provider is needed, aws_lc_rs feature is enabled by default
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        // Configure mTLS if client CA is provided
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

    // (2b) Add main guardrails server routes
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
        .route(
            &format!("{}/generation-detection", TEXT_API_PREFIX),
            post(generation_with_detection),
        )
        .route(
            &format!("{}/detection/content", TEXT_API_PREFIX),
            post(detection_content),
        )
        .route(
            &format!("{}/detection/context", TEXT_API_PREFIX),
            post(detect_context_documents),
        )
        .route(
            &format!("{}/detection/generated", TEXT_API_PREFIX),
            post(detect_generated),
        )
        .with_state(shared_state);

    // (2c) Generate main guardrails server handle based on whether TLS is needed
    let listener: TcpListener = TcpListener::bind(&http_addr)
        .await
        .unwrap_or_else(|_| panic!("failed to bind to {http_addr}"));
    let guardrails_handle = if arc_server_config.is_some() {
        // TLS
        // Use more low level server configuration than axum for configurability
        // Ref. https://github.com/tokio-rs/axum/blob/main/examples/low-level-rustls/src/main.rs
        info!("HTTPS server started on port {}", http_addr.port());
        let tls_acceptor = TlsAcceptor::from(arc_server_config.unwrap());
        tokio::spawn(async move {
            let graceful = hyper_util::server::graceful::GracefulShutdown::new();
            let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
            let mut signal = std::pin::pin!(shutdown_signal());
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

                let hyper_service =
                    hyper::service::service_fn(move |request: Request<Incoming>| {
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
                    debug!("Gracefully shutdown!");
                },
                () = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                    debug!("Waited 10 seconds for graceful shutdown, aborting...");
                }
            }
        })
    } else {
        // Non-TLS
        // Keep simple axum serve call for http version
        let http_server = axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(shutdown_signal());
        info!("HTTP server started on port {}", http_addr.port());
        tokio::task::spawn(async { http_server.await.expect("HTTP server crashed!") })
    };

    // (3) Launch each server as a separate task
    let (health_res, guardrails_res) = tokio::join!(health_handle, guardrails_handle);
    health_res.unwrap();
    guardrails_res.unwrap();
    info!("Shutdown complete for servers");
    Ok(())
}

async fn health() -> Result<impl IntoResponse, ()> {
    // NOTE: we are only adding the package information in the `health` endpoint to have this endpoint
    // provide a non empty 200 response. If we need to add more information regarding dependencies version
    // or such things, then we will add another `/info` endpoint accordingly. And those info
    // should not be added in `health` endpoint`
    let info_object = HashMap::from([(PACKAGE_NAME, PACKAGE_VERSION)]);
    Ok(Json(info_object).into_response())
}

async fn classification_with_gen(
    State(state): State<Arc<ServerState>>,
    WithRejection(Json(request), _): WithRejection<Json<models::GuardrailsHttpRequest>, Error>,
) -> Result<impl IntoResponse, Error> {
    let request_id = Uuid::new_v4();
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

async fn generation_with_detection(
    State(state): State<Arc<ServerState>>,
    WithRejection(Json(request), _): WithRejection<
        Json<models::GenerationWithDetectionHttpRequest>,
        Error,
    >,
) -> Result<impl IntoResponse, Error> {
    let request_id = Uuid::new_v4();
    request.validate()?;
    let task = GenerationWithDetectionTask::new(request_id, request);
    match state
        .orchestrator
        .handle_generation_with_detection(task)
        .await
    {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn stream_classification_with_gen(
    State(state): State<Arc<ServerState>>,
    WithRejection(Json(request), _): WithRejection<Json<models::GuardrailsHttpRequest>, Error>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let request_id = Uuid::new_v4();
    if let Err(error) = request.validate() {
        // Request validation failed, return stream with single error SSE event
        let error: Error = error.into();
        return Sse::new(
            stream::iter([Ok(Event::default()
                .event("error")
                .json_data(error.to_json())
                .unwrap())])
            .boxed(),
        );
    }
    let task = StreamingClassificationWithGenTask::new(request_id, request);
    let response_stream = state
        .orchestrator
        .handle_streaming_classification_with_gen(task)
        .await;
    // Convert response stream to a stream of SSE events
    let event_stream = response_stream
        .map(|message| match message {
            Ok(response) => Ok(Event::default()
                //.event("message") NOTE: per spec, should not be included for data-only message events
                .json_data(response)
                .unwrap()),
            Err(error) => {
                let error: Error = error.into();
                Ok(Event::default()
                    .event("error")
                    .json_data(error.to_json())
                    .unwrap())
            }
        })
        .boxed();
    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

async fn detection_content(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<models::TextContentDetectionHttpRequest>,
) -> Result<impl IntoResponse, Error> {
    let request_id = Uuid::new_v4();
    request.validate()?;
    let task = TextContentDetectionTask::new(request_id, request);
    match state.orchestrator.handle_text_content_detection(task).await {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn detect_context_documents(
    State(state): State<Arc<ServerState>>,
    WithRejection(Json(request), _): WithRejection<Json<models::ContextDocsHttpRequest>, Error>,
) -> Result<impl IntoResponse, Error> {
    let request_id = Uuid::new_v4();
    request.validate()?;
    let task = ContextDocsDetectionTask::new(request_id, request);
    match state
        .orchestrator
        .handle_context_documents_detection(task)
        .await
    {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn detect_generated(
    State(state): State<Arc<ServerState>>,
    WithRejection(Json(request), _): WithRejection<
        Json<models::DetectionOnGeneratedHttpRequest>,
        Error,
    >,
) -> Result<impl IntoResponse, Error> {
    let request_id = Uuid::new_v4();
    request.validate()?;
    let task = DetectionOnGenerationTask::new(request_id, request);
    match state
        .orchestrator
        .handle_generated_text_detection(task)
        .await
    {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => Err(error.into()),
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

// Ref. https://github.com/rustls/rustls/blob/main/examples/src/bin/tlsserver-mio.rs
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
    #[error("{0}")]
    ServiceUnavailable(String),
    #[error("unexpected error occured while processing request")]
    Unexpected,
    #[error(transparent)]
    JsonExtractorRejection(#[from] JsonRejection),
}

impl From<orchestrator::Error> for Error {
    fn from(value: orchestrator::Error) -> Self {
        use orchestrator::Error::*;
        match value {
            DetectorNotFound(_) => Self::NotFound(value.to_string()),
            DetectorRequestFailed { ref error, .. }
            | ChunkerRequestFailed { ref error, .. }
            | GenerateRequestFailed { ref error, .. }
            | TokenizeRequestFailed { ref error, .. } => match error.status_code() {
                StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                    Self::Validation(value.to_string())
                }
                StatusCode::NOT_FOUND => Self::NotFound(value.to_string()),
                StatusCode::SERVICE_UNAVAILABLE => Self::ServiceUnavailable(value.to_string()),
                _ => Self::Unexpected,
            },
            _ => Self::Unexpected,
        }
    }
}

impl Error {
    pub fn to_json(self) -> serde_json::Value {
        use Error::*;
        let (code, message) = match self {
            Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ServiceUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            Unexpected => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            JsonExtractorRejection(json_rejection) => match json_rejection {
                JsonRejection::JsonDataError(e) => {
                    // Get lower-level serde error message
                    let message = e.source().map(|e| e.to_string()).unwrap_or_default();
                    (e.status(), message)
                }
                _ => (json_rejection.status(), json_rejection.body_text()),
            },
        };
        serde_json::json!({
            "code": code.as_u16(),
            "details": message,
        })
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        use Error::*;
        let (code, message) = match self {
            Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ServiceUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            Unexpected => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            JsonExtractorRejection(json_rejection) => match json_rejection {
                JsonRejection::JsonDataError(e) => {
                    // Get lower-level serde error message
                    let message = e.source().map(|e| e.to_string()).unwrap_or_default();
                    (e.status(), message)
                }
                _ => (json_rejection.status(), json_rejection.body_text()),
            },
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
