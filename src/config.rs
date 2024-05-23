use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use tracing::debug;

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceConfig {
    pub hostname: String,
    pub port: Option<u16>,
    pub tls: Option<Tls>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Tls {
    Name(String),
    Config(TlsConfig),
}

#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    pub cert_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
    pub client_ca_cert_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenerationProvider {
    Tgis,
    Nlp,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenerationConfig {
    pub provider: GenerationProvider,
    pub service: ServiceConfig,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChunkerType {
    Sentence,
    All,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ChunkerConfig {
    pub r#type: ChunkerType,
    pub service: ServiceConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DetectorConfig {
    pub service: ServiceConfig,
    pub chunker_id: String,
    // Put threshold here _in_ config -> need to change type
    //pub config: HashMap<String, String>,
    // or threshold could be at this level but then would have to be optional
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrchestratorConfig {
    pub generation: GenerationConfig,
    pub chunkers: HashMap<String, ChunkerConfig>,
    pub detectors: HashMap<String, DetectorConfig>,
    pub tls: HashMap<String, TlsConfig>,
}

impl OrchestratorConfig {
    pub async fn load(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let s = tokio::fs::read_to_string(path)
            .await
            .unwrap_or_else(|error| {
                panic!("failed to read orchestrator config from {path:?}: {error}")
            });
        let mut config: OrchestratorConfig = serde_yml::from_str(&s)
            .unwrap_or_else(|error| panic!("invalid orchestrator config: {error}"));
        // Map tls name to tls config
        // TODO: do this cleaner
        let tls_configs = &config.tls;
        config.generation.service =
            service_tls_name_to_config(tls_configs, config.generation.service);
        config.chunkers = config
            .chunkers
            .into_iter()
            .map(|(name, mut config)| {
                config.service = service_tls_name_to_config(tls_configs, config.service);
                (name, config)
            })
            .collect::<HashMap<_, _>>();
        config.detectors = config
            .detectors
            .into_iter()
            .map(|(name, mut config)| {
                config.service = service_tls_name_to_config(tls_configs, config.service);
                (name, config)
            })
            .collect::<HashMap<_, _>>();
        debug!(?config, "loaded orchestrator config");
        config
    }

    fn _validate(&self) {
        todo!()
    }

    pub fn get_chunker_id(&self, detector_id: &str) -> Option<String> {
        self.detectors
            .get(detector_id)
            .map(|detector_config| detector_config.chunker_id.clone())
    }
}

fn service_tls_name_to_config(
    tls_configs: &HashMap<String, TlsConfig>,
    mut service: ServiceConfig,
) -> ServiceConfig {
    if let Some(Tls::Name(name)) = &service.tls {
        let tls_config = tls_configs.get(name).unwrap().clone(); // TODO: handle error
        service.tls = Some(Tls::Config(tls_config))
    }
    service
}

#[cfg(test)]
mod tests {
    use anyhow::Error;

    use super::*;

    #[test]
    fn test_deserialize_config() -> Result<(), Error> {
        let s = r#"
generation:
    provider: tgis
    service:
        hostname: localhost
        port: 8000
chunkers:
    sentence-en:
        type: sentence
        service:
            hostname: localhost
            port: 9000
    sentence-ja:
        type: sentence
        service:
            hostname: localhost
            port: 9000
detectors:
    hap-en:
        service:
            hostname: localhost
            port: 9000
        chunker_id: sentence-en
        config: {}
tls: {}
        "#;
        let config: OrchestratorConfig = serde_yml::from_str(s)?;
        assert!(config.chunkers.len() == 2 && config.detectors.len() == 1);
        Ok(())
    }
}
