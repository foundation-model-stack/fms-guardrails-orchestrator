
use std::{collections::HashMap, hash::Hash};

use tonic::transport::{
    server::RoutesBuilder, Certificate, ClientTlsConfig, Identity, Server, ServerTlsConfig,
};
use tokio::fs::read;

use crate::{clients::tgis::GenerationServicer, config::ServiceAddr};

// =========================================== Client Calls ==============================================

pub async fn configure_tgis(
    service_addr: ServiceAddr,
    default_target_port: u16,
) -> GenerationServicer {

    // NOTE: We only want to configure and connect to 1 TGIS "router"
    let model_map = HashMap::from([("tgis-all-models".to_owned(), service_addr.clone())]);

    // Configure TLS if requested
    let mut client_tls = service_addr.tls_enabled.then_some(ClientTlsConfig::new());
    if let Some(cert_path) = service_addr.tls_ca_path {
        // info!("Configuring TLS for outgoing connections to model servers");
        let cert_pem = load_pem(cert_path, "cert").await;
        let cert = Certificate::from_pem(cert_pem);
        client_tls = client_tls.map(|c| c.ca_certificate(cert));
    }
    let generation_servicer =
            GenerationServicer::new(default_target_port, client_tls.as_ref(), &model_map);
    generation_servicer.await
}


// =========================================== Util functions ==============================================


async fn load_pem(path: String, name: &str) -> Vec<u8> {
    read(&path)
        .await
        .unwrap_or_else(|_| panic!("couldn't load {name} from {path}"))
}
