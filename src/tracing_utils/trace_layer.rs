pub mod server {
    use crate::tracing_utils::current_trace_id;
    use axum::extract::Request;
    use axum::http::HeaderMap;
    use axum::response::Response;
    use std::time::Duration;
    use tower_http::classify::ServerErrorsFailureClass;
    use tracing::{debug, error, info, info_span, Span};

    pub fn request_span(request: &Request) -> Span {
        info_span!(
            "incoming orchestrator request",
            method = ?request.method(),
            host = request.uri().host().unwrap_or(""),
            path = request.uri().path(),
        )
    }

    pub fn on_request(request: &Request, span: &Span) {
        let _guard = span.enter();
        info!(
            trace_id = ?current_trace_id(),
            uri = ?request.uri(),
            "tracing incoming request",
        );
        info!(
            monotonic_counter.server_request_count = 1,
            method = ?request.method(),
            host = request.uri().host().unwrap_or(""),
            path = request.uri().path(),
        );
    }

    pub fn on_response(response: &Response, latency: Duration, span: &Span) {
        let _guard = span.enter();
        let trace_id = current_trace_id().to_string();
        let status_code = response.status().to_string();
        let latency_ms = latency.as_millis().to_string();
        info!(trace_id, status_code, latency_ms, "tracing server response");
        info!(
            monotonic_counter.server_response_count = 1,
            status_code, latency_ms,
        );
        info!(histogram.server_latency_ms = latency_ms, status_code,);

        if response.status().is_server_error() {
            // On every server error (HTTP 5xx) response
            // This should be caught by on_failure given the right classifier is used
            info!(
                monotonic_counter.server_5xx_error_response_count = 1,
                status_code, latency_ms,
            );
        } else if response.status().is_client_error() {
            // On every client error (HTTP 4xx) response
            info!(
                monotonic_counter.server_4xx_error_response_count = 1,
                status_code, latency_ms,
            );
        } else if response.status().is_success() {
            // On every successful (HTTP 2xx) response
            info!(
                monotonic_counter.server_success_response_count = 1,
                status_code, latency_ms,
            );
        }
    }

    pub fn on_eos(trailers: Option<&HeaderMap>, stream_duration: Duration, span: &Span) {
        let _guard = span.enter();
        let trace_id = current_trace_id().to_string();
        let stream_duration_ms = stream_duration.as_millis().to_string();

        info!(trace_id, stream_duration_ms, "server end of stream");
        debug!(?trailers, "server end of stream trailer headers");
        info!(
            monotonic_counter.server_stream_response_count = 1,
            stream_duration_ms,
        );
        info!(histogram.server_stream_duration_ms = stream_duration_ms);
    }

    pub fn on_failure(
        failure_classification: ServerErrorsFailureClass,
        latency: Duration,
        span: &Span,
    ) {
        let _guard = span.enter();
        let trace_id = current_trace_id().to_string();
        let latency_ms = latency.as_millis().to_string();

        let (status_code, error) = match failure_classification {
            ServerErrorsFailureClass::StatusCode(status_code) => {
                error!(
                    trace_id,
                    ?status_code,
                    latency_ms,
                    "server failed to handle request",
                );
                (Some(status_code), None)
            }
            ServerErrorsFailureClass::Error(error) => {
                error!(
                    trace_id,
                    latency_ms, "server failed to handle request - {}", error,
                );
                (None, Some(error))
            }
        };

        info!(
            monotonic_counter.server_request_failure_count = 1,
            latency_ms,
            ?status_code,
            ?error
        );
        info!(
            monotonic_counter.server_5xx_response_count = 1,
            latency_ms,
            ?status_code,
            ?error
        );
    }
}

pub mod client {}
