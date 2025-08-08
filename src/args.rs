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

use std::{fmt::Display, path::PathBuf};

use clap::Parser;
use tracing::{error, warn};
use url::Url;

use crate::{
    models::ValidationError,
    utils::trace::{DEFAULT_GRPC_OTLP_ENDPOINT, DEFAULT_HTTP_OTLP_ENDPOINT},
};

#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    #[clap(default_value = "8033", long, env)]
    pub http_port: u16,
    #[clap(default_value = "8034", long, env)]
    pub health_http_port: u16,
    #[clap(
        default_value = "config/config.yaml",
        long,
        env = "ORCHESTRATOR_CONFIG"
    )]
    pub config_path: PathBuf,
    #[clap(long, env)]
    pub tls_cert_path: Option<PathBuf>,
    #[clap(long, env)]
    pub tls_key_path: Option<PathBuf>,
    #[clap(long, env)]
    pub tls_client_ca_cert_path: Option<PathBuf>,
    #[clap(default_value = "false", long, env)]
    pub start_up_health_check: bool,
    #[clap(long, env, value_delimiter = ',')]
    pub otlp_export: Vec<OtlpExport>,
    #[clap(default_value_t = LogFormat::default(), long, env)]
    pub log_format: LogFormat,
    #[clap(default_value_t = false, long, short, env)]
    pub quiet: bool,
    #[clap(default_value = "fms_guardrails_orchestr8", long, env)]
    pub otlp_service_name: String,
    #[clap(long, env = "OTEL_EXPORTER_OTLP_ENDPOINT")]
    pub otlp_endpoint: Option<Url>,
    #[clap(long, env = "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")]
    pub otlp_traces_endpoint: Option<Url>,
    #[clap(long, env = "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT")]
    pub otlp_metrics_endpoint: Option<Url>,
    #[clap(
        default_value_t = OtlpProtocol::Grpc,
        long,
        env = "OTEL_EXPORTER_OTLP_PROTOCOL"
    )]
    pub otlp_protocol: OtlpProtocol,
    #[clap(long, env = "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL")]
    pub otlp_traces_protocol: Option<OtlpProtocol>,
    #[clap(long, env = "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL")]
    pub otlp_metrics_protocol: Option<OtlpProtocol>,
    // TODO: Add timeout and header OTLP variables
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OtlpExport {
    Traces,
    Metrics,
}

impl Display for OtlpExport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OtlpExport::Traces => write!(f, "traces"),
            OtlpExport::Metrics => write!(f, "metrics"),
        }
    }
}

impl From<String> for OtlpExport {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "traces" => OtlpExport::Traces,
            "metrics" => OtlpExport::Metrics,
            _ => panic!(
                "Invalid OTLP export type {s}, orchestrator only supports exporting traces and metrics via OTLP"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum OtlpProtocol {
    #[default]
    Grpc,
    Http,
}

impl Display for OtlpProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OtlpProtocol::Grpc => write!(f, "grpc"),
            OtlpProtocol::Http => write!(f, "http"),
        }
    }
}

impl From<String> for OtlpProtocol {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "grpc" => OtlpProtocol::Grpc,
            "http" => OtlpProtocol::Http,
            _ => {
                error!(
                    "Invalid OTLP protocol {}, defaulting to {}",
                    s,
                    OtlpProtocol::default()
                );
                OtlpProtocol::default()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum LogFormat {
    Compact,
    #[default]
    Full,
    Pretty,
    JSON,
}

impl Display for LogFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogFormat::Full => write!(f, "full"),
            LogFormat::Compact => write!(f, "compact"),
            LogFormat::Pretty => write!(f, "pretty"),
            LogFormat::JSON => write!(f, "json"),
        }
    }
}

impl From<String> for LogFormat {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "full" => LogFormat::Full,
            "compact" => LogFormat::Compact,
            "pretty" => LogFormat::Pretty,
            "json" => LogFormat::JSON,
            _ => {
                warn!(
                    "Invalid trace format {}, defaulting to {}",
                    s,
                    LogFormat::default()
                );
                LogFormat::default()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TracingConfig {
    pub service_name: String,
    pub traces: Option<(OtlpProtocol, String)>,
    pub metrics: Option<(OtlpProtocol, String)>,
    pub log_format: LogFormat,
    pub quiet: bool,
}

impl TryFrom<Args> for TracingConfig {
    type Error = ValidationError;

    fn try_from(args: Args) -> Result<Self, Self::Error> {
        use OtlpProtocol::*;
        let otlp_protocol = args.otlp_protocol;
        // Use provided otlp_traces_protocol or default to otlp_protocol
        let otlp_traces_protocol = args.otlp_traces_protocol.unwrap_or(otlp_protocol);
        // Use provided otlp_metrics_protocol or default to otlp_protocol
        let otlp_metrics_protocol = args.otlp_metrics_protocol.unwrap_or(otlp_protocol);

        // Validate provided endpoints
        if let Some(endpoint) = &args.otlp_endpoint
            && endpoint.path() != "/"
        {
            return Err(ValidationError::Invalid("invalid otlp_endpoint".into()));
        }
        if let Some(endpoint) = &args.otlp_traces_endpoint {
            match otlp_traces_protocol {
                Grpc => {
                    if endpoint.path() != "/" {
                        return Err(ValidationError::Invalid(
                            "invalid otlp_traces_endpoint for grpc protocol".into(),
                        ));
                    }
                }
                Http => {
                    if endpoint.path() != "/v1/traces" {
                        return Err(ValidationError::Invalid("invalid otlp_traces_endpoint for http protocol: path should be /v1/traces".into())
                        );
                    }
                }
            }
        }
        if let Some(endpoint) = &args.otlp_metrics_endpoint {
            match otlp_metrics_protocol {
                Grpc => {
                    if endpoint.path() != "/" {
                        return Err(ValidationError::Invalid(
                            "invalid otlp_metrics_endpoint for grpc protocol".into(),
                        ));
                    }
                }
                Http => {
                    if endpoint.path() != "/v1/metrics" {
                        return Err(ValidationError::Invalid(
                            "invalid otlp_metrics_endpoint for http protocol: path should be /v1/metrics".into(),
                        ));
                    }
                }
            }
        }

        // Use provided otlp_endpoint or default for protocol
        let otlp_endpoint = args.otlp_endpoint.unwrap_or(match otlp_protocol {
            Grpc => Url::parse(DEFAULT_GRPC_OTLP_ENDPOINT).unwrap(),
            Http => Url::parse(DEFAULT_HTTP_OTLP_ENDPOINT).unwrap(),
        });
        // Use provided otlp_traces_endpoint or build from otlp_endpoint
        let otlp_traces_endpoint =
            args.otlp_traces_endpoint
                .unwrap_or(match otlp_traces_protocol {
                    Grpc => otlp_endpoint.clone(),
                    Http => otlp_endpoint.clone().join("v1/traces").unwrap(),
                });
        // Use provided otlp_metrics_endpoint or build from otlp_endpoint
        let otlp_metrics_endpoint =
            args.otlp_metrics_endpoint
                .unwrap_or(match otlp_metrics_protocol {
                    Grpc => otlp_endpoint.clone(),
                    Http => otlp_endpoint.clone().join("v1/metrics").unwrap(),
                });

        Ok(TracingConfig {
            service_name: args.otlp_service_name,
            traces: match args.otlp_export.contains(&OtlpExport::Traces) {
                true => Some((otlp_traces_protocol, otlp_traces_endpoint.into())),
                false => None,
            },
            metrics: match args.otlp_export.contains(&OtlpExport::Metrics) {
                true => Some((otlp_metrics_protocol, otlp_metrics_endpoint.into())),
                false => None,
            },
            log_format: args.log_format,
            quiet: args.quiet,
        })
    }
}
