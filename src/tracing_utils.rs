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

use std::time::Duration;

use axum::extract::Request;
use axum::http::HeaderMap;
use axum::response::Response;
use opentelemetry::{
    global,
    metrics::MetricsError,
    trace::{TraceContextExt, TraceError, TracerProvider},
    KeyValue,
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    metrics::{
        reader::{DefaultAggregationSelector, DefaultTemporalitySelector},
        SdkMeterProvider,
    },
    runtime,
    trace::{Config, Sampler},
    Resource,
};
use tracing::{error, info, info_span, Span};
use tracing_opentelemetry::{MetricsLayer, OpenTelemetrySpanExt};
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Layer};

use crate::args::{LogFormat, OtlpProtocol, TracingConfig};

#[derive(Debug, thiserror::Error)]
pub enum TracingError {
    #[error("Error from tracing provider: {0}")]
    TraceError(#[from] TraceError),
    #[error("Error from metrics provider: {0}")]
    MetricsError(#[from] MetricsError),
}

/// Initializes an OpenTelemetry tracer provider with an OTLP export pipeline based on the
/// provided config.
fn init_tracer_provider(
    otlp_export_config: TracingConfig,
) -> Result<Option<opentelemetry_sdk::trace::TracerProvider>, TracingError> {
    if let Some((protocol, endpoint)) = otlp_export_config.traces {
        Ok(Some(
            match protocol {
                OtlpProtocol::Grpc => opentelemetry_otlp::new_pipeline().tracing().with_exporter(
                    opentelemetry_otlp::new_exporter()
                        .tonic()
                        .with_endpoint(endpoint)
                        .with_timeout(Duration::from_secs(3)),
                ),
                OtlpProtocol::Http => opentelemetry_otlp::new_pipeline().tracing().with_exporter(
                    opentelemetry_otlp::new_exporter()
                        .http()
                        .with_http_client(reqwest::Client::new())
                        .with_endpoint(endpoint)
                        .with_timeout(Duration::from_secs(3)),
                ),
            }
            .with_trace_config(
                Config::default()
                    .with_resource(Resource::new(vec![KeyValue::new(
                        "service.name",
                        otlp_export_config.service_name,
                    )]))
                    .with_sampler(Sampler::AlwaysOn),
            )
            .install_batch(runtime::Tokio)?,
        ))
    } else {
        Ok(None)
    }
}

/// Initializes an OpenTelemetry meter provider with an OTLP export pipeline based on the
/// provided config.
fn init_meter_provider(
    otlp_export_config: TracingConfig,
) -> Result<Option<SdkMeterProvider>, TracingError> {
    if let Some((protocol, endpoint)) = otlp_export_config.metrics {
        Ok(Some(
            match protocol {
                OtlpProtocol::Grpc => opentelemetry_otlp::new_pipeline()
                    .metrics(runtime::Tokio)
                    .with_exporter(
                        opentelemetry_otlp::new_exporter()
                            .tonic()
                            .with_endpoint(endpoint),
                    ),
                OtlpProtocol::Http => opentelemetry_otlp::new_pipeline()
                    .metrics(runtime::Tokio)
                    .with_exporter(
                        opentelemetry_otlp::new_exporter()
                            .http()
                            .with_http_client(reqwest::Client::new())
                            .with_endpoint(endpoint),
                    ),
            }
            .with_resource(Resource::new(vec![KeyValue::new(
                "service.name",
                otlp_export_config.service_name,
            )]))
            .with_timeout(Duration::from_secs(10))
            .with_period(Duration::from_secs(3))
            .with_aggregation_selector(DefaultAggregationSelector::new())
            .with_temporality_selector(DefaultTemporalitySelector::new())
            .build()?,
        ))
    } else {
        Ok(None)
    }
}

/// Initializes tracing for the orchestrator using the OpenTelemetry API/SDK and the `tracing`
/// crate. What telemetry is exported and to where is determined based on the provided config
pub fn init_tracing(
    tracing_config: TracingConfig,
) -> Result<impl FnOnce() -> Result<(), TracingError>, TracingError> {
    let mut layers = Vec::new();

    // TODO: Find a better way to only propagate errors from other crates
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or(EnvFilter::new("INFO"))
        .add_directive("ginepro=info".parse().unwrap())
        .add_directive("hyper=error".parse().unwrap())
        .add_directive("h2=error".parse().unwrap())
        .add_directive("trust_dns_resolver=error".parse().unwrap())
        .add_directive("trust_dns_proto=error".parse().unwrap())
        .add_directive("tower=error".parse().unwrap())
        .add_directive("tonic=error".parse().unwrap())
        .add_directive("reqwest=error".parse().unwrap());

    // Set up tracing layer with OTLP exporter
    let trace_provider = init_tracer_provider(tracing_config.clone())?;
    if let Some(tracer_provider) = trace_provider.clone() {
        global::set_tracer_provider(tracer_provider.clone());
        layers.push(
            tracing_opentelemetry::layer()
                .with_tracer(tracer_provider.tracer(tracing_config.service_name.clone()))
                .boxed(),
        );
    }

    // Set up metrics layer with OTLP exporter
    let meter_provider = init_meter_provider(tracing_config.clone())?;
    if let Some(meter_provider) = meter_provider.clone() {
        global::set_meter_provider(meter_provider.clone());
        layers.push(MetricsLayer::new(meter_provider).boxed());
    }

    // Set up formatted layer for logging to stdout
    // Because we use the `tracing` crate for logging, all logs are traces and will be exported
    // to OTLP if `--otlp-export=traces` is set.
    if !tracing_config.quiet {
        match tracing_config.log_format {
            LogFormat::Full => layers.push(tracing_subscriber::fmt::layer().boxed()),
            LogFormat::Compact => layers.push(tracing_subscriber::fmt::layer().compact().boxed()),
            LogFormat::Pretty => layers.push(tracing_subscriber::fmt::layer().pretty().boxed()),
            LogFormat::JSON => layers.push(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .boxed(),
            ),
        }
    }

    let subscriber = tracing_subscriber::registry().with(filter).with(layers);
    tracing::subscriber::set_global_default(subscriber).unwrap();

    if let Some(traces) = tracing_config.traces {
        info!(
            "OTLP tracing enabled: Exporting {} to {}",
            traces.0, traces.1
        );
    } else {
        info!("OTLP traces export disabled")
    }

    if let Some(metrics) = tracing_config.metrics {
        info!(
            "OTLP metrics enabled: Exporting {} to {}",
            metrics.0, metrics.1
        );
    } else {
        info!("OTLP metrics export disabled")
    }

    if !tracing_config.quiet {
        info!(
            "Stdout logging enabled with format {}",
            tracing_config.log_format
        );
    } else {
        info!("Stdout logging disabled"); // This will only be visible in traces
    }

    Ok(move || {
        global::shutdown_tracer_provider();
        if let Some(meter_provider) = meter_provider {
            meter_provider
                .shutdown()
                .map_err(TracingError::MetricsError)?;
        }
        Ok(())
    })
}

pub fn incoming_request_span(request: &Request) -> Span {
    info_span!(
        "incoming_orchestrator_http_request",
        request_method = request.method().to_string(),
        request_path = request.uri().path().to_string(),
        response_status_code = tracing::field::Empty,
        request_duration_ms = tracing::field::Empty,
        stream_response = tracing::field::Empty,
        stream_response_event_count = tracing::field::Empty,
        stream_response_error_count = tracing::field::Empty,
        stream_response_duration_ms = tracing::field::Empty,
    )
}

pub fn on_incoming_request(request: &Request, span: &Span) {
    let _guard = span.enter();
    info!(
        "incoming request to {} {} with trace_id {}",
        request.method(),
        request.uri().path(),
        span.context().span().span_context().trace_id().to_string()
    );
    info!(
        monotonic_counter.incoming_request_count = 1,
        request_method = request.method().as_str(),
        request_path = request.uri().path()
    );
}

pub fn on_outgoing_response(response: &Response, latency: Duration, span: &Span) {
    let _guard = span.enter();
    span.record("response_status_code", response.status().as_u16());
    span.record("request_duration_ms", latency.as_millis());

    info!(
        "response {} for request with with trace_id {} generated in {} ms",
        &response.status(),
        span.context().span().span_context().trace_id().to_string(),
        latency.as_millis()
    );

    // On every response
    info!(
        monotonic_counter.handled_request_count = 1,
        response_status = response.status().as_u16(),
        request_duration = latency.as_millis()
    );
    info!(
        histogram.service_request_duration = latency.as_millis(),
        response_status = response.status().as_u16()
    );

    if response.status().is_server_error() {
        // On every server error (HTTP 5xx) response
        info!(
            monotonic_counter.server_error_response_count = 1,
            response_status = response.status().as_u16(),
            request_duration = latency.as_millis()
        );
    } else if response.status().is_client_error() {
        // On every client error (HTTP 4xx) response
        info!(
            monotonic_counter.client_error_response_count = 1,
            response_status = response.status().as_u16(),
            request_duration = latency.as_millis()
        );
    } else if response.status().is_success() {
        // On every successful (HTTP 2xx) response
        info!(
            monotonic_counter.success_response_count = 1,
            response_status = response.status().as_u16(),
            request_duration = latency.as_millis()
        );
    } else {
        error!(
            "unexpected response status code: {}",
            response.status().as_u16()
        );
    }
}

pub fn on_outgoing_eos(trailers: Option<&HeaderMap>, stream_duration: Duration, span: &Span) {
    let _guard = span.enter();

    span.record("stream_response", true);
    span.record("stream_response_duration_ms", stream_duration.as_millis());

    info!(
        "stream response for request with trace_id {} closed after {} ms with trailers: {:?}",
        span.context().span().span_context().trace_id().to_string(),
        stream_duration.as_millis(),
        trailers
    );
    info!(
        monotonic_counter.service_stream_response_count = 1,
        stream_duration = stream_duration.as_millis()
    );
    info!(monotonic_histogram.service_stream_response_duration = stream_duration.as_millis());
}
