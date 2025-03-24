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

use axum::{extract::Request, http::HeaderMap, response::Response};
use opentelemetry::{
    KeyValue, global,
    trace::{TraceContextExt, TraceError, TraceId, TracerProvider},
};
use opentelemetry_http::{HeaderExtractor, HeaderInjector};
use opentelemetry_otlp::{MetricExporter, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    Resource,
    metrics::{MetricError, PeriodicReader, SdkMeterProvider},
    propagation::TraceContextPropagator,
    runtime,
    trace::Sampler,
};
use tracing::{Span, error, info, info_span};
use tracing_opentelemetry::{MetricsLayer, OpenTelemetrySpanExt};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt};

use crate::{
    args::{LogFormat, OtlpProtocol, TracingConfig},
    clients::http::TracedResponse,
};

#[derive(Debug, thiserror::Error)]
pub enum TracingError {
    #[error("Error from tracing provider: {0}")]
    TraceError(#[from] TraceError),
    #[error("Error from metrics provider: {0}")]
    MetricError(#[from] MetricError),
}

fn resource(tracing_config: TracingConfig) -> Resource {
    Resource::new(vec![KeyValue::new(
        "service.name",
        tracing_config.service_name,
    )])
}

/// Initializes an OpenTelemetry tracer provider with an OTLP export pipeline based on the
/// provided config.
fn init_tracer_provider(
    tracing_config: TracingConfig,
) -> Result<Option<opentelemetry_sdk::trace::TracerProvider>, TracingError> {
    if let Some((protocol, endpoint)) = tracing_config.clone().traces {
        let timeout = Duration::from_secs(3);
        let exporter = match protocol {
            OtlpProtocol::Grpc => SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .with_timeout(timeout)
                .build()?,
            OtlpProtocol::Http => SpanExporter::builder()
                .with_http()
                .with_http_client(reqwest::Client::new())
                .with_endpoint(endpoint)
                .with_timeout(timeout)
                .build()?,
        };
        Ok(Some(
            opentelemetry_sdk::trace::TracerProvider::builder()
                .with_batch_exporter(exporter, runtime::Tokio)
                .with_resource(resource(tracing_config))
                .with_sampler(Sampler::AlwaysOn)
                .build(),
        ))
    } else if !tracing_config.quiet {
        // We still need a tracing provider as long as we are logging in order to enable any
        // trace-sensitive logs, such as any mentions of a request's trace_id.
        Ok(Some(
            opentelemetry_sdk::trace::TracerProvider::builder()
                .with_resource(resource(tracing_config))
                .with_sampler(Sampler::AlwaysOn)
                .build(),
        ))
    } else {
        Ok(None)
    }
}

/// Initializes an OpenTelemetry meter provider with an OTLP export pipeline based on the
/// provided config.
fn init_meter_provider(
    tracing_config: TracingConfig,
) -> Result<Option<SdkMeterProvider>, TracingError> {
    if let Some((protocol, endpoint)) = tracing_config.clone().metrics {
        // Note: DefaultAggregationSelector removed from OpenTelemetry SDK as of 0.26.0
        // as custom aggregation should be available in Views. Cumulative temporality is default.
        let timeout = Duration::from_secs(10);
        let exporter = match protocol {
            OtlpProtocol::Grpc => MetricExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .with_timeout(timeout)
                .build()?,
            OtlpProtocol::Http => MetricExporter::builder()
                .with_http()
                .with_http_client(reqwest::Client::new())
                .with_endpoint(endpoint)
                .with_timeout(timeout)
                .build()?,
        };
        let reader = PeriodicReader::builder(exporter, runtime::Tokio)
            .with_interval(Duration::from_secs(3))
            .build();
        Ok(Some(
            SdkMeterProvider::builder()
                .with_resource(resource(tracing_config))
                .with_reader(reader)
                .build(),
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
    global::set_text_map_propagator(TraceContextPropagator::new());

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
                .map_err(TracingError::MetricError)?;
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
        trace_id = span.context().span().span_context().trace_id().to_string(),
        method = %request.method(),
        path = %request.uri().path(),
        monotonic_counter.incoming_request_count = 1,
        "started processing orchestrator request",
    );
}

pub fn on_outgoing_response(response: &Response, latency: Duration, span: &Span) {
    let _guard = span.enter();
    span.record("response_status_code", response.status().as_u16());
    span.record("request_duration_ms", latency.as_millis());

    // On every response
    // Note: tracing_opentelemetry expects u64/f64 for histograms but as_millis returns u128
    info!(
        trace_id = span.context().span().span_context().trace_id().to_string(),
        status = %response.status(),
        duration_ms = %latency.as_millis(),
        monotonic_counter.handled_request_count = 1,
        histogram.service_request_duration = latency.as_millis() as u64,
        "finished processing orchestrator request"
    );

    if response.status().is_server_error() {
        // On every server error (HTTP 5xx) response
        info!(monotonic_counter.server_error_response_count = 1);
    } else if response.status().is_client_error() {
        // On every client error (HTTP 4xx) response
        // Named so that this does not get mixed up with orchestrator
        // client response metrics
        info!(monotonic_counter.client_app_error_response_count = 1);
    } else if response.status().is_success() {
        // On every successful (HTTP 2xx) response
        info!(monotonic_counter.success_response_count = 1);
    } else {
        error!(
            "unexpected response status code: {}",
            response.status().as_u16()
        );
    }
}

pub fn on_outgoing_eos(_trailers: Option<&HeaderMap>, stream_duration: Duration, span: &Span) {
    let _guard = span.enter();

    span.record("stream_response", true);
    span.record("stream_response_duration_ms", stream_duration.as_millis());

    info!(
        trace_id = span.context().span().span_context().trace_id().to_string(),
        monotonic_counter.service_stream_response_count = 1,
        histogram.service_stream_response_duration = stream_duration.as_millis() as u64,
        stream_duration = stream_duration.as_millis(),
        "end of orchestrator stream"
    );
}

/// Injects the `traceparent` header into the header map from the current tracing span context.
/// Also injects empty `tracestate` header by default. This can be used to propagate
/// vendor-specific trace context.
/// Used by both gRPC and HTTP requests since `tonic::Metadata` uses `http::HeaderMap`.
/// See https://www.w3.org/TR/trace-context/#trace-context-http-headers-format.
pub fn with_traceparent_header(ctx: &opentelemetry::Context, headers: HeaderMap) -> HeaderMap {
    global::get_text_map_propagator(|propagator| {
        let mut headers = headers.clone();
        // Injects current `traceparent` (and by default empty `tracestate`)
        propagator.inject_context(ctx, &mut HeaderInjector(&mut headers));
        headers
    })
}

/// Extracts the `traceparent` header from an HTTP response's headers and uses it to set the current
/// tracing span context (i.e. use `traceparent` as parent to the current span).
/// Defaults to using the current context when no `traceparent` is found.
/// See https://www.w3.org/TR/trace-context/#trace-context-http-headers-format.
pub fn trace_context_from_http_response(span: &Span, response: &TracedResponse) {
    let curr_trace = span.context().span().span_context().trace_id();
    let ctx = global::get_text_map_propagator(|propagator| {
        // Returns the current context if no `traceparent` is found
        propagator.extract(&HeaderExtractor(response.headers()))
    });
    if ctx.span().span_context().trace_id() == curr_trace {
        span.set_parent(ctx);
    }
}

/// Extracts the `traceparent` header from a gRPC response's metadata and uses it to set the current
/// tracing span context (i.e. use `traceparent` as parent to the current span).
/// Defaults to using the current context when no `traceparent` is found.
/// See https://www.w3.org/TR/trace-context/#trace-context-http-headers-format.
pub fn trace_context_from_grpc_response<T>(span: &Span, response: &tonic::Response<T>) {
    let curr_trace = span.context().span().span_context().trace_id();
    let ctx = global::get_text_map_propagator(|propagator| {
        let metadata = response.metadata().clone();
        // Returns the current context if no `traceparent` is found
        propagator.extract(&HeaderExtractor(&metadata.into_headers()))
    });
    if ctx.span().span_context().trace_id() == curr_trace {
        span.set_parent(ctx);
    }
}

/// Returns the `trace_id` of the current span according to the global tracing subscriber.
pub fn current_trace_id() -> TraceId {
    Span::current().context().span().span_context().trace_id()
}
