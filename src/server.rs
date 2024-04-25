use crate::{
    clients::nlp::NlpServicer, config::{self, DetectorMap, OrchestratorConfig}, models::{self, HttpValidationError}, orchestrator, utils, ErrorResponse, GuardrailsResponse};


use core::panic;
use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::State, http::StatusCode, response::IntoResponse, routing::{get, post}, Json, Router
};
// sse -> server side events
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use serde::Serialize;
use tokio::signal;
use tracing::info;
use std::convert::Infallible;

// ========================================== Constants and Dummy Variables ==========================================
const API_PREFIX: &'static str = r#"/api/v1/task"#;
const TGIS_PORT: u16 = 8033;
const DEFAULT_CAIKIT_NLP_PORT: u16 = 8085;

// TODO: Change with real object
struct InnerResponse {
    sample: bool
}
struct SampleResponse {
    response: InnerResponse
}

// TODO: Dummy streaming response object
#[derive(Serialize)]
pub(crate) struct StreamResponse {
    pub generated_text: String,
    pub processed_index: i32,
}

const DUMMY_RESPONSE: [&'static str; 9] = ["This", "is", "very", "good", "news,", "streaming", "is", "working", "!"];

// ========================================== Handler functions ==========================================


/// Server shared state
#[derive(Clone)]
pub(crate) struct ServerState {
    // pub tgis_servicer: GenerationServicer,
    pub caikit_nlp_servicer: NlpServicer,
    pub detector_config: DetectorMap
}

/// Run the orchestrator server
#[allow(clippy::too_many_arguments)]
pub async fn run(
    rest_addr: SocketAddr,
    // tls_key_pair: Option<(String, String)>,
    orchestrator_config: config::OrchestratorConfig,
) {

    // TODO: Configure TLS for this server if requested
    // TODO: How to share orchestrator_config across handler

    // Configure TGIS
    // let tgis_servicer = utils::configure_tgis(
    //     orchestrator_config.tgis_config,
    //     TGIS_PORT
    // ).await;

    // Configure Caikit NLP
    let caikit_nlp_servicer = utils::configure_nlp(
       orchestrator_config.caikit_nlp_config,
       DEFAULT_CAIKIT_NLP_PORT
    ).await;

    // Add server and configs to shared state
    let shared_state = Arc::new(ServerState {
        caikit_nlp_servicer,
        detector_config: orchestrator_config.detector_config
    });

    // Build and await on the HTTP server
    let app = Router::new()
        .route("/health", get(health))
        .route(&format!("{}/classification-with-text-generation", API_PREFIX), post(classification_with_generation))
        .route(&format!("{}/server-streaming-classification-with-text-generation", API_PREFIX), post(stream_classification_with_gen))
        .with_state(shared_state);

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

// #[debug_handler]
// TODO: Improve Bad Request error handling by implementing Validate middleware
async fn classification_with_generation(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<models::GuardrailsHttpRequest>) -> Json<GuardrailsResponse> {

    info!("Unary classification call");
    if let Ok(detector_hashmaps) = orchestrator::preprocess_detector_map(state.detector_config.clone()) {
        let response = orchestrator::do_unary_tasks(payload, detector_hashmaps, state.caikit_nlp_servicer.clone()).await;
        return Json(GuardrailsResponse::SuccessfulResponse(response))
    }
    // Dummy error for now
    Json(GuardrailsResponse::ValidationError(HttpValidationError { detail: None }))
}


// #[debug_handler]
async fn stream_classification_with_gen(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<models::GuardrailsHttpRequest>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {

    info!("Streaming classification call");
    let on_message_callback = |stream_token: models::ClassifiedGeneratedTextStreamResult| {
        let event = Event::default();
        event.json_data(stream_token).unwrap()
    };

    // TODO: check if input guardrails is required and if so call detectors

    let guardrails_model_id = "en_syntax_slate.38m.hap".to_string();

    let token_class_result =
        utils::call_nlp_token_classification(
            payload.clone().inputs,
            guardrails_model_id,
            None,
            state.caikit_nlp_servicer.clone());

    // How to convert non stream to stream.
    // let token_class_to_stream = Stream<Item = Result<TokenClassificationResults, ErrorResponse>>::new( {
    //     stream::unfold((), |()| async { Some((token_class_result.await, ())) })
    // });
    // let token_class_strm = stream::once(token_class_result);

    let _ = async_stream::stream! {
        let result = token_class_result.await;
        match result {
            // TODO: Add logic to parse and handle token_class_result properly
            Ok(value) => {
                print!("Response from token class result: {:?}", value);
                // TODO: Add logic to parse classification response and send appropriate event back
                // based on output
                Ok(Event::default())
            },
            _ => Err(Event::default())

        }
    };


    let response_stream =
            utils::call_nlp_text_gen_stream(
                Json(payload),
                state.caikit_nlp_servicer.clone(),
                on_message_callback
            );

    Sse::new(response_stream.await).keep_alive(KeepAlive::default())

}


impl IntoResponse for ErrorResponse {
    fn into_response(self) -> axum::response::Response {
        match self.error {
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "Error handling not implemented".to_string())
        }.into_response()
    }
}

async fn handle_error(error: ErrorResponse) -> (StatusCode, String) {
    match error.error {
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "Error handling not implemented".to_string())
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

