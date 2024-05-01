use std::collections::HashMap;

use futures::future::try_join_all;
use ginepro::LoadBalancedChannel;
use url::Url;

use crate::config::{ServiceConfig, Tls};

pub mod detector;
pub use detector::DetectorClient;

pub mod tgis;
pub use tgis::TgisClient;

pub mod nlp;
pub use nlp::NlpClient;

pub const DEFAULT_TGIS_PORT: u16 = 8033;
pub const DEFAULT_CAIKIT_NLP_PORT: u16 = 8085;
pub const DEFAULT_DETECTOR_PORT: u16 = 8080;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error(transparent)]
    ReqwestError(#[from] reqwest::Error),
    #[error(transparent)]
    TonicError(#[from] tonic::Status),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
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

    pub fn client(&self) -> reqwest::Client {
        self.client.clone()
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
) -> Result<HashMap<String, HttpClient>, Error> {
    let clients = config
        .iter()
        .map(|(name, service_config)| async move {
            let port = service_config.port.unwrap_or(default_port);
            let mut base_url = Url::parse(&service_config.hostname).unwrap();
            base_url.set_port(Some(port)).unwrap();
            let mut builder = reqwest::ClientBuilder::new();
            if let Some(Tls::Config(tls_config)) = &service_config.tls {
                let cert_path = tls_config.cert_path.as_ref().unwrap().as_path();
                let cert_pem = tokio::fs::read(cert_path).await?;
                //let key_pem = tokio::fs::read(key_path).await?;
                let identity = reqwest::Identity::from_pem(&cert_pem)?;
                builder = builder.use_rustls_tls().identity(identity);
            }
            let client = builder.build()?;
            let client = HttpClient::new(base_url, client);
            Ok((name.clone(), client)) as Result<(String, HttpClient), Error>
        })
        .collect::<Vec<_>>();
    Ok(try_join_all(clients).await?.into_iter().collect())
}

async fn create_grpc_clients<C>(
    default_port: u16,
    config: &[(String, ServiceConfig)],
    new: fn(LoadBalancedChannel) -> C,
) -> Result<HashMap<String, C>, Error> {
    let clients = config
        .iter()
        .map(|(name, service_config)| async move {
            let mut builder = LoadBalancedChannel::builder((
                service_config.hostname.clone(),
                service_config.port.unwrap_or(default_port),
            ));
            let client_tls_config = if let Some(Tls::Config(tls_config)) = &service_config.tls {
                let cert_path = tls_config.cert_path.as_ref().unwrap().as_path();
                let key_path = tls_config.key_path.as_ref().unwrap().as_path();
                let cert_pem = tokio::fs::read(cert_path).await?;
                let key_pem = tokio::fs::read(key_path).await?;
                let identity = tonic::transport::Identity::from_pem(cert_pem, key_pem);
                let mut client_tls_config =
                    tonic::transport::ClientTlsConfig::new().identity(identity);
                if let Some(client_ca_cert_path) = &tls_config.client_ca_cert_path {
                    let client_ca_cert_pem = tokio::fs::read(client_ca_cert_path).await?;
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
            let channel = builder.channel().await.unwrap(); // TODO: handle error
            Ok((name.clone(), new(channel))) as Result<(String, C), Error>
        })
        .collect::<Vec<_>>();
    Ok(try_join_all(clients).await?.into_iter().collect())
}
