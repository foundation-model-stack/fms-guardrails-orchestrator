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

use opentelemetry::{
    global,
    metrics::MetricsError,
    trace::{TraceError, TracerProvider},
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
use tracing::{error, info};
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Layer};

use crate::{
    args::{OtlpExportConfig, OtlpProtocol, StdoutLogConfig},
};

#[derive(Debug, thiserror::Error)]
pub enum TracingError {
    #[error("Error from tracing provider: {0}")]
    TraceError(#[from] TraceError),
    #[error("Error from metrics provider: {0}")]
    MetricsError(#[from] MetricsError),
}

fn init_tracer_provider(
    otlp_export_config: OtlpExportConfig,
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
                        .with_resource(Resource::new(vec![KeyValue::new("service.name", otlp_export_config.service_name)]))
                        .with_sampler(Sampler::AlwaysOn),
                )
                .install_batch(runtime::Tokio)?,
        ))
    } else {
        Ok(None)
    }
}

fn init_meter_provider(
    otlp_export_config: OtlpExportConfig,
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
                .with_resource(Resource::new(vec![KeyValue::new("service.name", otlp_export_config.service_name)]))
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

pub fn init_tracer(
    otlp_export_config: OtlpExportConfig,
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
    let trace_provider = init_tracer_provider(otlp_export_config.clone())?;
    if let Some(tracer_provider) = trace_provider.clone() {
        global::set_tracer_provider(tracer_provider.clone());
        layers.push(
            tracing_opentelemetry::layer()
                .with_tracer(tracer_provider.tracer(otlp_export_config.service_name.clone()))
                .boxed(),
        );
    }

    // Set up metrics layer with OTLP exporter
    let meter_provider = init_meter_provider(otlp_export_config.clone())?;
    if let Some(meter_provider) = meter_provider.clone() {
        global::set_meter_provider(meter_provider.clone());
        layers.push(MetricsLayer::new(meter_provider).boxed());
    }

    // Set up formatted layer for logging to stdout
    // Because we use the `tracing` crate for logging, all logs are traces and will be exported
    // to OTLP if `--otlp-export=traces` is set.
    match otlp_export_config.stdout_log {
        StdoutLogConfig::Full => layers.push(tracing_subscriber::fmt::layer().boxed()),
        StdoutLogConfig::Compact => layers.push(tracing_subscriber::fmt::layer().compact().boxed()),
        StdoutLogConfig::Pretty => layers.push(tracing_subscriber::fmt::layer().pretty().boxed()),
        StdoutLogConfig::JSON => layers.push(
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .boxed(),
        ),
        StdoutLogConfig::None => (),
    }

    let subscriber = tracing_subscriber::registry().with(filter).with(layers);
    tracing::subscriber::set_global_default(subscriber).unwrap();

    if let Some(traces) = otlp_export_config.traces {
        info!(
            "OTLP tracing enabled: Exporting {} to {}",
            traces.0, traces.1
        );
    } else {
        info!("OTLP traces export disabled")
    }

    if let Some(metrics) = otlp_export_config.metrics {
        info!(
            "OTLP metrics enabled: Exporting {} to {}",
            metrics.0, metrics.1
        );
    } else {
        info!("OTLP metrics export disabled")
    }

    if otlp_export_config.stdout_log != StdoutLogConfig::None {
        info!(
            "Stdout logging enabled with format {}",
            otlp_export_config.stdout_log
        );
    } else {
        info!("Stdout logging disabled");
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
