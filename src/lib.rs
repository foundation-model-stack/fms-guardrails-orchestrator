// Declare modules

use anyhow::Context;
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


#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
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


// TODO: Should we change below and separate out to create separate one for TGIS / Detectors etc

async fn create_clients<C>(
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
        .expect("Error creating upstream service clients")
        .into_iter()
        .collect()
}