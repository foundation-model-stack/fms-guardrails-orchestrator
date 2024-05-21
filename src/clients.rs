#![allow(dead_code)]
use std::{collections::HashMap, time::Duration};

use futures::future::join_all;
use ginepro::LoadBalancedChannel;
use reqwest::StatusCode;
use url::Url;

use crate::config::{ServiceConfig, Tls};

pub mod chunker;
pub use chunker::ChunkerClient;

pub mod detector;
pub use detector::DetectorClient;

pub mod tgis;
pub use tgis::TgisClient;

pub mod nlp;
pub use nlp::NlpClient;

pub const DEFAULT_TGIS_PORT: u16 = 8033;
pub const DEFAULT_CAIKIT_NLP_PORT: u16 = 8085;
pub const DEFAULT_CHUNKER_PORT: u16 = 8085;
pub const DEFAULT_DETECTOR_PORT: u16 = 8080;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Client errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}", .0.message())]
    Grpc(#[from] tonic::Status),
    #[error("{0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid model id: {model_id}")]
    InvalidModelId { model_id: String },
}

impl Error {
    /// Returns status code.
    pub fn status_code(&self) -> StatusCode {
        use tonic::Code::*;
        match self {
            // Return equivalent http status code for grpc status code
            Error::Grpc(error) => match error.code() {
                InvalidArgument => StatusCode::BAD_REQUEST,
                Internal => StatusCode::INTERNAL_SERVER_ERROR,
                NotFound => StatusCode::NOT_FOUND,
                DeadlineExceeded => StatusCode::REQUEST_TIMEOUT,
                Unimplemented => StatusCode::NOT_IMPLEMENTED,
                Unauthenticated => StatusCode::UNAUTHORIZED,
                PermissionDenied => StatusCode::FORBIDDEN,
                Ok => StatusCode::OK,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            // Return http status code for error responses
            // and 500 for other errors
            Error::Http(error) => match error.status() {
                Some(code) => code,
                None => StatusCode::INTERNAL_SERVER_ERROR,
            },
            // Return 422 for invalid model id
            Error::InvalidModelId { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        }
    }

    /// Returns true for validation-type errors (400/422) and false for other types.
    pub fn is_validation_error(&self) -> bool {
        matches!(
            self.status_code(),
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
        )
    }
}

#[derive(Clone)]
pub enum GenerationClient {
    Tgis(TgisClient),
    Nlp(NlpClient),
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
            let mut builder = reqwest::ClientBuilder::new()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .timeout(DEFAULT_REQUEST_TIMEOUT);
            if let Some(Tls::Config(tls_config)) = &service_config.tls {
                let cert_path = tls_config.cert_path.as_ref().unwrap().as_path();
                let cert_pem = tokio::fs::read(cert_path).await.unwrap_or_else(|error| {
                    panic!("error reading cert from {cert_path:?}: {error}")
                });
                let identity = reqwest::Identity::from_pem(&cert_pem)
                    .unwrap_or_else(|error| panic!("error creating identity: {error}"));
                builder = builder.use_rustls_tls().identity(identity);
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
            let mut builder = LoadBalancedChannel::builder((
                service_config.hostname.clone(),
                service_config.port.unwrap_or(default_port),
            ))
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .timeout(DEFAULT_REQUEST_TIMEOUT);
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
                    tonic::transport::ClientTlsConfig::new().identity(identity);
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
