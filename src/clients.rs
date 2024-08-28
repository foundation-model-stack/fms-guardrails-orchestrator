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

#![allow(dead_code)]
// Import error for adding `source` trait
use std::{collections::HashMap, error::Error as _, pin::Pin, time::Duration};

use futures::{future::join_all, Stream};
use ginepro::LoadBalancedChannel;
use reqwest::StatusCode;
use tokio::{fs::File, io::AsyncReadExt};
use tracing::{debug, error};
use url::Url;

use crate::{
    config::{ServiceConfig, Tls},
    orchestrator::HealthStatus,
};

pub mod chunker;
pub use chunker::ChunkerClient;

pub mod detector;
pub use detector::DetectorClient;

pub mod generation;
pub use generation::GenerationClient;

pub mod tgis;
pub use tgis::TgisClient;

pub mod nlp;
pub use nlp::NlpClient;

pub const DEFAULT_TGIS_PORT: u16 = 8033;
pub const DEFAULT_CAIKIT_NLP_PORT: u16 = 8085;
pub const DEFAULT_CHUNKER_PORT: u16 = 8085;
pub const DEFAULT_DETECTOR_PORT: u16 = 8080;
pub const COMMON_ROUTER_KEY: &str = "common-router";
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_REQUEST_TIMEOUT_SEC: u64 = 600;

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

/// Client errors.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("{}", .message)]
    Grpc { code: StatusCode, message: String },
    #[error("{}", .message)]
    Http { code: StatusCode, message: String },
    #[error("model not found: {model_id}")]
    ModelNotFound { model_id: String },
    #[error("health check request failed for model `{model_id}`: health status is unknown")]
    HealthCheckRequestFailed { model_id: String },
}

impl Error {
    /// Returns status code.
    pub fn status_code(&self) -> StatusCode {
        match self {
            // Return equivalent http status code for grpc status code
            Error::Grpc { code, .. } => *code,
            // Return http status code for error responses
            // and 500 for other errors
            Error::Http { code, .. } => *code,
            // Return 404 for model not found
            Error::ModelNotFound { .. } => StatusCode::NOT_FOUND,
            // Return 500 for health check failures
            Error::HealthCheckRequestFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        // Log lower level source of error.
        // Examples:
        // 1. client error (Connect) // Cases like connection error, wrong port etc.
        // 2. client error (SendRequest) // Cases like cert issues
        error!(
            "http request failed. Source: {}",
            value.source().unwrap().to_string()
        );
        // Return http status code for error responses
        // and 500 for other errors
        let code = match value.status() {
            Some(code) => code,
            None => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::Http {
            code,
            message: value.to_string(),
        }
    }
}

impl From<tonic::Status> for Error {
    fn from(value: tonic::Status) -> Self {
        use tonic::Code::*;
        // Return equivalent http status code for grpc status code
        let code = match value.code() {
            InvalidArgument => StatusCode::BAD_REQUEST,
            Internal => StatusCode::INTERNAL_SERVER_ERROR,
            NotFound => StatusCode::NOT_FOUND,
            DeadlineExceeded => StatusCode::REQUEST_TIMEOUT,
            Unimplemented => StatusCode::NOT_IMPLEMENTED,
            Unauthenticated => StatusCode::UNAUTHORIZED,
            PermissionDenied => StatusCode::FORBIDDEN,
            Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Ok => StatusCode::OK,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::Grpc {
            code,
            message: value.message().to_string(),
        }
    }
}

pub trait HealthCheck {
    async fn check(&self) -> Result<HealthStatus, Error>;
}

pub trait HealthProbe {
    async fn ready(&self) -> Result<HashMap<String, HealthStatus>, Error>;
    async fn live(&self) -> Result<HashMap<String, HealthStatus>, Error> {
        unimplemented!()
    }
}

#[derive(Clone)]
pub struct HttpClient {
    base_url: Url,
    client: reqwest::Client,
}

impl HttpClient {
    pub fn new(base_url: Url, client: reqwest::Client) -> Self {
        Self { base_url, client }
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub fn health_endpoint(&self) -> Url {
        let mut url = self.base_url.clone();
        url.set_path("/health");
        url
    }
}

impl HealthCheck for HttpClient {
    async fn check(&self) -> Result<HealthStatus, Error> {
        self.get(self.health_endpoint().as_str())
            .send()
            .await
            .map(|response| Ok(response.status().into()))
            .unwrap_or_else(|error| {
                if error.is_status() {
                    Ok(error.status().unwrap().into())
                } else {
                    Err(Error::HealthCheckRequestFailed {
                        model_id: self.base_url().as_str().to_string(),
                    })
                }
            })
    }
}

impl std::ops::Deref for HttpClient {
    type Target = reqwest::Client;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

pub async fn create_http_clients(
    default_port: u16,
    config: &[(String, ServiceConfig)],
) -> HashMap<String, HttpClient> {
    let clients = config
        .iter()
        .map(|(name, service_config)| async move {
            let port = service_config.port.unwrap_or(default_port);
            let mut base_url = Url::parse(&service_config.hostname).unwrap();
            base_url.set_port(Some(port)).unwrap();
            let request_timeout = Duration::from_secs(
                service_config
                    .request_timeout
                    .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SEC),
            );
            let mut builder = reqwest::ClientBuilder::new()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .timeout(request_timeout);
            if let Some(Tls::Config(tls_config)) = &service_config.tls {
                let mut cert_buf = Vec::new();
                let cert_path = tls_config.cert_path.as_ref().unwrap().as_path();
                File::open(cert_path)
                    .await
                    .unwrap_or_else(|error| {
                        panic!("error reading cert from {cert_path:?}: {error}")
                    })
                    .read_to_end(&mut cert_buf)
                    .await
                    .unwrap();

                if let Some(key_path) = &tls_config.key_path {
                    File::open(key_path)
                        .await
                        .unwrap_or_else(|error| {
                            panic!("error reading key from {key_path:?}: {error}")
                        })
                        .read_to_end(&mut cert_buf)
                        .await
                        .unwrap();
                }
                let identity = reqwest::Identity::from_pem(&cert_buf).unwrap_or_else(|error| {
                    panic!("error parsing bundled client certificate: {error}")
                });

                builder = builder.use_rustls_tls().identity(identity);

                debug!(?tls_config.insecure);
                builder = builder.danger_accept_invalid_certs(tls_config.insecure.unwrap_or(false));

                if let Some(client_ca_cert_path) = &tls_config.client_ca_cert_path {
                    let ca_cert =
                        tokio::fs::read(client_ca_cert_path)
                            .await
                            .unwrap_or_else(|error| {
                                panic!("error reading cert from {client_ca_cert_path:?}: {error}")
                            });
                    let cacert = reqwest::Certificate::from_pem(&ca_cert)
                        .unwrap_or_else(|error| panic!("error parsing ca cert: {error}"));
                    builder = builder.add_root_certificate(cacert)
                }
            }
            let client = builder
                .build()
                .unwrap_or_else(|error| panic!("error creating http client for {name}: {error}"));
            let client = HttpClient::new(base_url, client);
            (name.clone(), client)
        })
        .collect::<Vec<_>>();
    join_all(clients).await.into_iter().collect()
}

async fn create_grpc_clients<C>(
    default_port: u16,
    config: &[(String, ServiceConfig)],
    new: fn(LoadBalancedChannel) -> C,
) -> HashMap<String, C> {
    let clients = config
        .iter()
        .map(|(name, service_config)| async move {
            let request_timeout = Duration::from_secs(service_config.request_timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT_SEC));
            let mut builder = LoadBalancedChannel::builder((
                service_config.hostname.clone(),
                service_config.port.unwrap_or(default_port),
            ))
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .timeout(request_timeout);

            let client_tls_config = if let Some(Tls::Config(tls_config)) = &service_config.tls {
                let cert_path = tls_config.cert_path.as_ref().unwrap().as_path();
                let key_path = tls_config.key_path.as_ref().unwrap().as_path();
                let cert_pem = tokio::fs::read(cert_path)
                    .await
                    .unwrap_or_else(|error| panic!("error reading cert from {cert_path:?}: {error}"));
                let key_pem = tokio::fs::read(key_path)
                    .await
                    .unwrap_or_else(|error| panic!("error reading key from {key_path:?}: {error}"));
                let identity = tonic::transport::Identity::from_pem(cert_pem, key_pem);
                let mut client_tls_config =
                    tonic::transport::ClientTlsConfig::new().identity(identity).with_native_roots().with_webpki_roots();
                if let Some(client_ca_cert_path) = &tls_config.client_ca_cert_path {
                    let client_ca_cert_pem = tokio::fs::read(client_ca_cert_path)
                        .await
                        .unwrap_or_else(|error| {
                            panic!("error reading client ca cert from {client_ca_cert_path:?}: {error}")
                        });
                    client_tls_config = client_tls_config.ca_certificate(
                        tonic::transport::Certificate::from_pem(client_ca_cert_pem),
                    );
                }
                Some(client_tls_config)
            } else {
                None
            };
            if let Some(client_tls_config) = client_tls_config {
                builder = builder.with_tls(client_tls_config);
            }
            let channel = builder.channel().await.unwrap_or_else(|error| panic!("error creating grpc client for {name}: {error}"));
            (name.clone(), new(channel))
        })
        .collect::<Vec<_>>();
    join_all(clients).await.into_iter().collect()
}
