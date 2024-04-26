// Declare modules

use anyhow::Context;
use axum::http::StatusCode;
use futures::future::try_join_all;
use ginepro::LoadBalancedChannel;

use std::{collections::HashMap};
use tonic::transport::ClientTlsConfig;

use serde::{Serialize, Deserialize};

pub mod clients;
pub mod config;
pub mod models;
pub mod orchestrator;
pub mod server;
pub mod utils;
mod pb;

use reqwest::{Client as RestClient, Certificate as ReqwestCertificate, Url};


#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum GuardrailsResponse {
    /// Successful Response
    SuccessfulResponse
    (models::ClassifiedGeneratedTextResult)
    ,
    /// Validation Error
    ValidationError
    (models::HttpValidationError)
}

// Struct meant to hold client configuration
// It currently stores url and Client object
#[derive(Debug, Default, Clone)]
pub struct RestClientConfig {
    url: String,
    client: RestClient
}


// TODO: Should we change below and separate out to create separate one for TGIS / Detectors etc

async fn create_grpc_clients<C>(
    default_target_port: u16,
    client_tls: Option<&ClientTlsConfig>,
    model_map: &HashMap<String, config::ServiceAddr>,
    new: fn(LoadBalancedChannel) -> C,
) -> HashMap<String, C> {
    let clients = model_map
        .iter()
        .map(|(name, service_addr)| async move {
            // info!("Configuring client for model name: [{name}]");
            // Build a load-balanced channel given a service name and a port.
            let mut builder = LoadBalancedChannel::builder((
                service_addr.hostname.clone(),
                service_addr.port.unwrap_or(default_target_port),
            ));
            if let Some(tls_config) = client_tls {
                builder = builder.with_tls(tls_config.clone());
            }
            let channel = builder
                .channel()
                .await
                .context(format!("Channel failed for service {name}"))?;
            Ok((name.clone(), new(channel))) as Result<(String, C), anyhow::Error>
        })
        .collect::<Vec<_>>();
    try_join_all(clients)
        .await
        .expect("Error creating upstream service for gRPC clients")
        .into_iter()
        .collect()
}

// Function to create rest clients given a model_map containing
// model name mapped to their client service address
async fn create_rest_clients (
    default_target_port: u16,
    model_map: &HashMap<String, config::ServiceAddr>
 ) -> HashMap<String, RestClientConfig> {

    let clients = model_map
        .iter()
        .map(|(name, service_addr)| async move {

            let mut client_builder = reqwest::ClientBuilder::new();

            // Check if tls is enabled for this model
            if service_addr.tls_enabled == true {
                if !service_addr.tls_ca_path.is_none() {
                    let tls_ca_path: &String = service_addr.tls_ca_path.as_ref().unwrap();
                    let cert_pem = utils::load_pem(tls_ca_path.clone(), "cert").await;
                    let cert = ReqwestCertificate::from_pem(&cert_pem)?;
                    client_builder = client_builder.add_root_certificate(cert).use_rustls_tls();
                }
            }
            // TODO: create timeouts
            let client = client_builder.build()?;

            let mut url_obj = Url::parse(service_addr.hostname.as_str().as_ref()).unwrap();

            if service_addr.port.is_some() {
                url_obj.set_port(service_addr.port);
            }
            let client_config = RestClientConfig {
                url: url_obj.as_str().into(),
                client: client
            };

            Ok((name.clone(), client_config)) as Result<(String, RestClientConfig), anyhow::Error>

        })
        .collect::<Vec<_>>();
    try_join_all(clients)
        .await
        .expect("Error creating upstream service for REST clients")
        .into_iter()
        .collect()

}