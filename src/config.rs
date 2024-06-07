use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use tracing::debug;

/// Configuration for service needed for
/// orchestrator to communicate with it
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ServiceConfig {
    pub hostname: String,
    pub port: Option<u16>,
    pub tls: Option<Tls>,
}

/// TLS provider
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Tls {
    Name(String),
    Config(TlsConfig),
}

/// Client TLS configuration
#[derive(Clone, Debug, Default, Deserialize)]
pub struct TlsConfig {
    pub cert_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
    pub client_ca_cert_path: Option<PathBuf>,
}

/// Generation service provider
#[cfg_attr(test, derive(Default))]
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenerationProvider {
    #[cfg_attr(test, default)]
    Tgis,
    Nlp,
}

/// Generate service configuration
#[cfg_attr(test, derive(Default))]
#[derive(Clone, Debug, Deserialize)]
pub struct GenerationConfig {
    /// Generation service provider
    pub provider: GenerationProvider,
    /// Generation service connection information
    pub service: ServiceConfig,
}

/// Chunker parser type
#[cfg_attr(test, derive(Default))]
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChunkerType {
    #[cfg_attr(test, default)]
    Sentence,
    All,
}

/// Configuration for each chunker
#[cfg_attr(test, derive(Default))]
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct ChunkerConfig {
    /// Chunker type
    pub r#type: ChunkerType,
    /// Chunker service connection information
    pub service: ServiceConfig,
}

/// Configuration for each detector
#[derive(Clone, Debug, Default, Deserialize)]
pub struct DetectorConfig {
    /// Detector service connection information
    pub service: ServiceConfig,
    /// ID of chunker that this detector will use
    pub chunker_id: String,
    /// Default threshold with which to filter detector results by score
    pub default_threshold: f32,
}

/// Overall orchestrator server configuration
#[cfg_attr(test, derive(Default))]
#[derive(Clone, Debug, Deserialize)]
pub struct OrchestratorConfig {
    /// Generation service and associated configuration
    pub generation: GenerationConfig,
    /// Chunker services and associated configurations
    pub chunkers: HashMap<String, ChunkerConfig>,
    /// Detector services and associated configurations
    pub detectors: HashMap<String, DetectorConfig>,
    /// Map of TLS connections, allowing reuse across services
    /// that may require the same TLS information
    pub tls: HashMap<String, TlsConfig>,
}

impl OrchestratorConfig {
    /// Load overall orchestrator server configuration
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

    /// Get ID of chunker associated with a particular detector
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
impl Default for Tls {
    fn default() -> Self {
        Tls::Name("dummy_tls".to_string())
    }
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
        default_threshold: 0.5
tls: {}
        "#;
        let config: OrchestratorConfig = serde_yml::from_str(s)?;
        assert!(config.chunkers.len() == 2 && config.detectors.len() == 1);
        Ok(())
    }
}
