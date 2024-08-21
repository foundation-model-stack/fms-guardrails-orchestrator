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

use serde::Deserialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tracing::{debug, error, warn};

use crate::{clients::chunker::DEFAULT_MODEL_ID, server};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("failed to read config file from {path:?}: {error}")]
    FailedToReadConfigFile { path: String, error: String },
    #[error("failed to serialize config file {path}: {error}")]
    FailedToSerializeConfigFile { path: String, error: String },
    #[error("TLS config `{name}` not found for service {host}:{port} in config")]
    TlsConfigNotFound {
        name: String,
        host: String,
        port: String,
    },
    #[error("no detectors provided in config")]
    NoDetectorsConfigured,
    #[error("config for detector `{detector}` has an unknown chunker_id `{chunker}`")]
    DetectorChunkerNotFound { detector: String, chunker: String },
}

impl From<Error> for server::Error {
    fn from(error: Error) -> Self {
        match error {
            Error::FailedToReadConfigFile { .. }
            | Error::FailedToSerializeConfigFile { .. }
            | Error::TlsConfigNotFound { .. }
            | Error::NoDetectorsConfigured
            | Error::DetectorChunkerNotFound { .. } => {
                server::Error::ConfigurationFailed(error.to_string())
            }
        }
    }
}

impl From<serde_yml::Error> for Error {
    fn from(error: serde_yml::Error) -> Self {
        Error::FailedToSerializeConfigFile {
            path: "".to_string(),
            error: error.to_string(),
        }
    }
}

/// Configuration for service needed for
/// orchestrator to communicate with it
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ServiceConfig {
    /// Hostname for service
    pub hostname: String,
    /// Port for service
    pub port: Option<u16>,
    /// Timeout in seconds for request to be handled
    pub request_timeout: Option<u64>,
    /// TLS provider info
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
#[cfg_attr(test, derive(Default))]
#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfig {
    pub cert_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
    pub client_ca_cert_path: Option<PathBuf>,
    pub insecure: Option<bool>,
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
    pub default_threshold: f64,
}

/// Overall orchestrator server configuration
#[cfg_attr(test, derive(Default))]
#[derive(Clone, Debug, Deserialize)]
pub struct OrchestratorConfig {
    /// Generation service and associated configuration, can be omitted if configuring for generation is not wanted
    pub generation: Option<GenerationConfig>,
    /// Chunker services and associated configurations, if omitted the default value "whole_doc_chunker" is used
    pub chunkers: Option<HashMap<String, ChunkerConfig>>,
    /// Detector services and associated configurations
    pub detectors: HashMap<String, DetectorConfig>,
    /// Map of TLS connections, allowing reuse across services
    /// that may require the same TLS information
    pub tls: Option<HashMap<String, TlsConfig>>,
}

impl OrchestratorConfig {
    /// Load overall orchestrator server configuration
    pub async fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let mut config = Self::from_file(path).await?;
        debug!(?config, "loaded orchestrator config");

        Self::map_tls_configs(&mut config)?;
        config.validate()
    }

    async fn from_file(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let s = tokio::fs::read_to_string(path).await.map_err(|error| {
            Error::FailedToReadConfigFile {
                path: path.to_str().unwrap().to_string(),
                error: error.to_string(),
            }
        })?;
        serde_yml::from_str(&s).map_err(|error| Error::FailedToSerializeConfigFile {
            path: path.to_str().unwrap().to_string(),
            error: error.to_string(),
        })
    }

    fn map_tls_configs(config: &mut Self) -> Result<(), Error> {
        let tls_configs = &config.tls;
        match config.generation {
            Some(ref mut generation) => {
                generation.service =
                    Self::tls_name_to_config(tls_configs, generation.clone().service)?;
            }
            None => warn!("no generation service configuration provided"),
        }

        match config.chunkers {
            Some(ref mut chunkers) => {
                for (_, chunker) in chunkers.iter_mut() {
                    chunker.service =
                        Self::tls_name_to_config(tls_configs, chunker.clone().service)?;
                }
            }
            None => warn!("no chunker service configurations provided"),
        }

        for (_, detector) in config.detectors.iter_mut() {
            detector.service = Self::tls_name_to_config(tls_configs, detector.clone().service)?;
        }

        Ok(())
    }

    fn validate(&self) -> Result<Self, Error> {
        if self.detectors.is_empty() {
            Err(Error::NoDetectorsConfigured)
        } else {
            for (_, detector) in self.detectors.iter() {
                if !self
                    .clone()
                    .chunkers
                    .unwrap()
                    .contains_key(&detector.chunker_id)
                    && detector.chunker_id != DEFAULT_MODEL_ID
                {
                    return Err(Error::DetectorChunkerNotFound {
                        detector: detector.clone().service.hostname,
                        chunker: detector.chunker_id.clone(),
                    });
                }
            }
            Ok(self.clone())
        }
    }

    /// Get ID of chunker associated with a particular detector
    pub fn get_chunker_id(&self, detector_id: &str) -> Option<String> {
        self.detectors
            .get(detector_id)
            .map(|detector_config| detector_config.chunker_id.clone())
    }

    fn tls_name_to_config(
        tls_configs: &Option<HashMap<String, TlsConfig>>,
        mut service: ServiceConfig,
    ) -> Result<ServiceConfig, Error> {
        match &service.tls {
            Some(Tls::Name(name)) => {
                if let Some(tls_configs) = tls_configs {
                    let tls_config = tls_configs
                        .get(name)
                        .ok_or(Error::TlsConfigNotFound {
                            name: name.clone(),
                            host: service.clone().hostname,
                            port: service.clone().port.unwrap_or(0).to_string(),
                        })?
                        .clone();
                    service.tls = Some(Tls::Config(tls_config));
                    Ok(service)
                } else {
                    Err(Error::TlsConfigNotFound {
                        name: name.clone(),
                        host: service.clone().hostname,
                        port: service.clone().port.unwrap_or(0).to_string(),
                    })
                }
            }
            _ => Ok(service),
        }
    }
}

#[cfg(test)]
impl Default for Tls {
    fn default() -> Self {
        Tls::Name("dummy_tls".to_string())
    }
}

#[cfg(test)]
mod tests {
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
        assert!(
            config
                .chunkers
                .expect("chunkers should have been configured")
                .len()
                == 2
                && config.detectors.len() == 1
        );
        Ok(())
    }

    #[test]
    fn test_deserialize_config_detector_tls_signed() -> Result<(), Error> {
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
    hap:
        service:
            hostname: localhost
            port: 9000
            tls: detector
        chunker_id: sentence-en
        default_threshold: 0.5
tls:
    detector:
        cert_path: /certs/client.pem
        "#;
        let config: OrchestratorConfig = serde_yml::from_str(s)?;
        assert!(
            config
                .chunkers
                .expect("chunkers should have been configured")
                .len()
                == 2
                && config.detectors.len() == 1
        );
        assert!(
            config
                .tls
                .as_ref()
                .expect("tls should have been configured")
                .len()
                == 1
                && config.tls.as_ref().unwrap().contains_key("detector")
        );
        Ok(())
    }

    #[test]
    fn test_deserialize_config_detector_tls_insecure() -> Result<(), Error> {
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
    hap:
        service:
            hostname: localhost
            port: 9000
            tls: detector
        chunker_id: sentence-en
        default_threshold: 0.5
tls:
    detector:
        client_ca_cert_path: /certs/ca.pem
        cert_path: /certs/client.pem
        key_path: /certs/client-key.pem
        insecure: true
        "#;
        let config: OrchestratorConfig = serde_yml::from_str(s)?;
        assert!(
            config
                .chunkers
                .expect("chunkers should have been configured")
                .len()
                == 2
                && config.detectors.len() == 1
        );
        assert!(
            config
                .tls
                .as_ref()
                .expect("tls should have been configured")
                .len()
                == 1
                && config
                    .tls
                    .as_ref()
                    .unwrap()
                    .get("detector")
                    .unwrap()
                    .insecure
                    == Some(true)
        );
        Ok(())
    }
}

//
