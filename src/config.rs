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

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::Deserialize;
use tracing::{debug, error, info, warn};

use crate::{
    clients::{chunker::DEFAULT_CHUNKER_ID, is_valid_hostname},
    utils::one_or_many,
};

/// Default allowed headers to passthrough to clients.
const DEFAULT_ALLOWED_HEADERS: &[&str] = &[];

/// Default number of detector requests to send concurrently for a task.
const fn default_detector_concurrent_requests() -> usize {
    5
}
/// Default number of chunker requests to send concurrently for a task.
const fn default_chunker_concurrent_requests() -> usize {
    5
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read config from `{path}`: {error}")]
    FailedToReadConfigFile { path: String, error: std::io::Error },
    #[error("invalid config file: {0}")]
    InvalidConfigFile(serde_yml::Error),
    #[error("tls config `{name}` not found for service `{host}:{port}`")]
    TlsConfigNotFound {
        name: String,
        host: String,
        port: String,
    },
    #[error("no detectors configured")]
    NoDetectorsConfigured,
    #[error("chunker `{chunker_id}` not found for detector `{detector_id}`")]
    DetectorChunkerNotFound {
        detector_id: String,
        chunker_id: String,
    },
    #[error("invalid generation provider: {0}")]
    InvalidGenerationProvider(String),
    #[error("invalid hostname: {0}")]
    InvalidHostname(String),
}

/// Configuration for service needed for
/// orchestrator to communicate with it
#[derive(Default, Clone, Debug, Deserialize)]
pub struct ServiceConfig {
    /// Hostname for service
    pub hostname: String,
    /// Port for service
    pub port: Option<u16>,
    /// Timeout in seconds for request to be handled
    pub request_timeout: Option<u64>,
    /// TLS provider info
    pub tls: Option<Tls>,
    /// gRPC probe interval in seconds
    pub grpc_dns_probe_interval: Option<u64>,
    /// Resolution strategy
    pub resolution_strategy: Option<String>,
    /// Resolution strategy timeout in seconds
    pub resolution_strategy_timeout: Option<u64>,
    /// Max retries for client calls [currently only for grpc generation]
    pub max_retries: Option<usize>,
}

impl ServiceConfig {
    pub fn new(hostname: String, port: u16) -> Self {
        Self {
            hostname,
            port: Some(port),
            request_timeout: None,
            tls: None,
            grpc_dns_probe_interval: None,
            resolution_strategy: None,
            resolution_strategy_timeout: None,
            max_retries: None,
        }
    }
}

/// TLS provider
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Tls {
    Name(String),
    Config(TlsConfig),
}

/// Client TLS configuration
#[derive(Default, Clone, Debug, Deserialize)]
pub struct TlsConfig {
    pub cert_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
    pub client_ca_cert_path: Option<PathBuf>,
    pub insecure: Option<bool>,
}

/// Generation service provider
#[derive(Default, Clone, Copy, Debug, Deserialize)]
pub enum GenerationProvider {
    #[default]
    #[serde(rename = "tgis")]
    Tgis,
    #[serde(rename = "nlp")]
    Nlp,
}

/// Generation service configuration
#[derive(Default, Clone, Debug, Deserialize)]
pub struct GenerationConfig {
    /// Generation service provider
    pub provider: GenerationProvider,
    /// Generation service connection information
    pub service: ServiceConfig,
}

/// OpenAI service configuration
#[derive(Default, Clone, Debug, Deserialize)]
pub struct OpenAiConfig {
    /// Generation service connection information
    pub service: ServiceConfig,
    /// Generation health service connection information
    pub health_service: Option<ServiceConfig>,
}

/// Chunker parser type
#[derive(Default, Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChunkerType {
    #[default]
    Sentence,
    All,
}

/// Configuration for each chunker
#[derive(Default, Clone, Debug, Deserialize)]
pub struct ChunkerConfig {
    /// Chunker type
    pub r#type: ChunkerType,
    /// Chunker service connection information
    pub service: ServiceConfig,
}

/// Configuration for each detector
#[derive(Default, Clone, Debug, Deserialize)]
pub struct DetectorConfig {
    /// Detector service connection information
    pub service: ServiceConfig,
    /// Detector health service connection information
    pub health_service: Option<ServiceConfig>,
    /// ID of chunker that this detector will use
    pub chunker_id: String,
    /// Default threshold with which to filter detector results by score
    pub default_threshold: f64,
    /// Type of detection this detector performs
    #[serde(rename = "type", deserialize_with = "one_or_many")]
    pub r#type: Vec<DetectorType>,
}

#[derive(Default, Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DetectorType {
    #[default]
    TextContents,
    TextGeneration,
    TextChat,
    TextContextDoc,
}

/// Overall orchestrator server configuration
#[derive(Clone, Debug, Deserialize)]
pub struct OrchestratorConfig {
    /// Generation service and associated configuration, can be omitted if configuring for generation is not wanted
    pub generation: Option<GenerationConfig>,
    /// OpenAI service and associated configuration, can be omitted if configuring for chat generation is not wanted
    #[serde(alias = "chat_generation")]
    #[serde(alias = "chat_completions")]
    pub openai: Option<OpenAiConfig>,
    /// Chunker services and associated configurations, if omitted the default value "whole_doc_chunker" is used
    pub chunkers: Option<HashMap<String, ChunkerConfig>>,
    /// Detector services and associated configurations
    pub detectors: HashMap<String, DetectorConfig>,
    /// Map of TLS connections, allowing reuse across services
    /// that may require the same TLS information
    pub tls: Option<HashMap<String, TlsConfig>>,
    // List of header keys allowed to be passed to downstream servers
    #[serde(default)]
    pub passthrough_headers: HashSet<String>,
    /// Number of detector requests to send concurrently for a task.
    #[serde(default = "default_detector_concurrent_requests")]
    pub detector_concurrent_requests: usize,
    /// Number of chunker requests to send concurrently for a task.
    #[serde(default = "default_chunker_concurrent_requests")]
    pub chunker_concurrent_requests: usize,
}

impl OrchestratorConfig {
    /// Loads config
    pub async fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let config_yaml = tokio::fs::read_to_string(path).await.map_err(|error| {
            Error::FailedToReadConfigFile {
                path: path.to_string_lossy().to_string(),
                error,
            }
        })?;
        // TODO: Remove if conditions once aliases are deprecated
        if config_yaml.contains("chat_generation") {
            warn!(
                "`chat_generation` is deprecated and will be removed in 1.0. Rename it to `openai`."
            )
        }
        if config_yaml.contains("chat_completions") {
            warn!(
                "`chat_completions` is deprecated and will be removed in 1.0. Rename it to `openai`."
            )
        }
        let mut config: OrchestratorConfig =
            serde_yml::from_str(&config_yaml).map_err(Error::InvalidConfigFile)?;
        debug!(?config, "loaded orchestrator config");

        if config.generation.is_none() {
            warn!("no generation config provided");
        }
        if config.chunkers.is_none() {
            warn!("no chunker configs provided");
        }

        if config.passthrough_headers.is_empty() {
            info!("No allowed headers specified");
        }

        // Add default headers to allowed_headers list
        debug!(
            "Adding default headers: [{}]. ",
            DEFAULT_ALLOWED_HEADERS.join(", ")
        );

        // Lowercase all header for case-insensitive comparison
        config.passthrough_headers = config
            .passthrough_headers
            .into_iter()
            .map(|h| h.to_lowercase())
            .collect::<HashSet<String>>();

        config
            .passthrough_headers
            .extend(DEFAULT_ALLOWED_HEADERS.iter().map(|h| h.to_lowercase()));

        config.apply_named_tls_configs()?;
        config.validate()?;

        Ok(config)
    }

    /// Applies named TLS configs to services.
    fn apply_named_tls_configs(&mut self) -> Result<(), Error> {
        if let Some(tls_configs) = &self.tls {
            // Generation
            if let Some(generation) = &mut self.generation {
                apply_named_tls_config(&mut generation.service, tls_configs)?;
            }
            // Open AI
            if let Some(openai) = &mut self.openai {
                apply_named_tls_config(&mut openai.service, tls_configs)?;
            }
            // Chunkers
            if let Some(chunkers) = &mut self.chunkers {
                for chunker in chunkers.values_mut() {
                    apply_named_tls_config(&mut chunker.service, tls_configs)?;
                }
            }
            // Detectors
            for detector in self.detectors.values_mut() {
                apply_named_tls_config(&mut detector.service, tls_configs)?;
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), Error> {
        // Detectors are configured
        if self.detectors.is_empty() {
            return Err(Error::NoDetectorsConfigured);
        }

        // Apply validation rules
        self.validate_generation_config()?;
        self.validate_openai_configs()?;
        self.validate_detector_configs()?;
        self.validate_chunker_configs()?;

        Ok(())
    }

    /// Validates generation config.
    fn validate_generation_config(&self) -> Result<(), Error> {
        if let Some(generation) = &self.generation {
            // Hostname is valid
            if !is_valid_hostname(&generation.service.hostname) {
                return Err(Error::InvalidHostname(
                    "`generation` has an invalid hostname".into(),
                ));
            }
        }
        Ok(())
    }

    /// Validates chat generation config.
    fn validate_openai_configs(&self) -> Result<(), Error> {
        if let Some(openai) = &self.openai {
            // Hostname is valid
            if !is_valid_hostname(&openai.service.hostname) {
                return Err(Error::InvalidHostname(
                    "`openai` has an invalid hostname".into(),
                ));
            }
        }

        Ok(())
    }

    /// Validates detector configs.
    fn validate_detector_configs(&self) -> Result<(), Error> {
        for (detector_id, detector) in &self.detectors {
            // Hostname is valid
            if !is_valid_hostname(&detector.service.hostname) {
                return Err(Error::InvalidHostname(format!(
                    "detector `{detector_id}` has an invalid hostname"
                )));
            }
            // Chunker is valid
            let valid_chunker = detector.chunker_id == DEFAULT_CHUNKER_ID
                || self
                    .chunkers
                    .as_ref()
                    .is_some_and(|chunkers| chunkers.contains_key(&detector.chunker_id));
            if !valid_chunker {
                return Err(Error::DetectorChunkerNotFound {
                    detector_id: detector_id.clone(),
                    chunker_id: detector.chunker_id.clone(),
                });
            }
        }
        Ok(())
    }

    /// Validates chunker configs.
    fn validate_chunker_configs(&self) -> Result<(), Error> {
        if let Some(chunkers) = &self.chunkers {
            for (chunker_id, chunker) in chunkers {
                // Hostname is valid
                if !is_valid_hostname(&chunker.service.hostname) {
                    return Err(Error::InvalidHostname(format!(
                        "chunker `{chunker_id}` has an invalid hostname"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Get ID of chunker associated with a particular detector
    pub fn get_chunker_id(&self, detector_id: &str) -> Option<String> {
        self.detectors
            .get(detector_id)
            .map(|detector_config| detector_config.chunker_id.clone())
    }

    /// Gets a chunker config.
    pub fn chunker(&self, chunker_id: &str) -> Option<&ChunkerConfig> {
        if let Some(chunkers) = &self.chunkers {
            chunkers.get(chunker_id)
        } else {
            None
        }
    }

    /// Gets a detector config.
    pub fn detector(&self, detector_id: &str) -> Option<&DetectorConfig> {
        self.detectors.get(detector_id)
    }
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            generation: None,
            openai: None,
            chunkers: None,
            detectors: HashMap::default(),
            tls: None,
            passthrough_headers: HashSet::default(),
            detector_concurrent_requests: default_detector_concurrent_requests(),
            chunker_concurrent_requests: default_chunker_concurrent_requests(),
        }
    }
}

/// Applies named TLS config to a service.
fn apply_named_tls_config(
    service: &mut ServiceConfig,
    tls_configs: &HashMap<String, TlsConfig>,
) -> Result<(), Error> {
    if let Some(Tls::Name(name)) = &service.tls {
        let tls_config = tls_configs
            .get(name)
            .ok_or(Error::TlsConfigNotFound {
                name: name.clone(),
                host: service.hostname.clone(),
                port: service.port.unwrap_or(0).to_string(),
            })?
            .clone();
        service.tls = Some(Tls::Config(tls_config));
    }
    Ok(())
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
        type: text_contents
        service:
            hostname: localhost
            port: 9000
        chunker_id: sentence-en
        default_threshold: 0.5
tls: {}
        "#;
        let config: OrchestratorConfig = serde_yml::from_str(s).unwrap();
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
        type: text_contents
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
        let config: OrchestratorConfig = serde_yml::from_str(s).unwrap();
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
        type: text_contents
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
        let config: OrchestratorConfig = serde_yml::from_str(s).unwrap();
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

    #[test]
    fn test_deserialize_config_no_detectors() {
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
detectors: {}
tls: {}
        "#;
        let mut config: OrchestratorConfig = serde_yml::from_str(s).unwrap();
        config
            .apply_named_tls_configs()
            .expect("Apply named TLS configs should have succeeded");
        let error = config
            .validate()
            .expect_err("Config should not have been validated");
        assert!(matches!(error, Error::NoDetectorsConfigured))
    }

    #[test]
    fn test_deserialize_config_tls_not_found() {
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
        type: text_contents
        service:
            hostname: localhost
            port: 9000
            tls: notadetector
        chunker_id: sentence-en
        default_threshold: 0.5
tls:
    detector:
        client_ca_cert_path: /certs/ca.pem
        cert_path: /certs/client.pem
        key_path: /certs/client-key.pem
        "#;
        let mut config: OrchestratorConfig = serde_yml::from_str(s).unwrap();
        let error = config
            .apply_named_tls_configs()
            .expect_err("Apply named TLS configs should have failed");
        assert!(matches!(error, Error::TlsConfigNotFound { .. }))
    }

    #[test]
    fn test_deserialize_config_chunker_found() {
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
        type: text_contents
        service:
            hostname: localhost
            port: 9000
            tls: detector
        chunker_id: sentence-fr
        default_threshold: 0.5
tls:
    detector:
        client_ca_cert_path: /certs/ca.pem
        cert_path: /certs/client.pem
        key_path: /certs/client-key.pem
        "#;
        let mut config: OrchestratorConfig = serde_yml::from_str(s).unwrap();
        config
            .apply_named_tls_configs()
            .expect("Apply named TLS configs should have succeeded");
        config
            .validate()
            .expect_err("Config should not have been validated");
    }

    #[test]
    fn test_passthrough_headers_empty_config() -> Result<(), Error> {
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
detectors:
    hap:
        type: text_contents
        service:
            hostname: localhost
            port: 9000
            tls: detector
        chunker_id: sentence-fr
        default_threshold: 0.5
        "#;
        let config: OrchestratorConfig = serde_yml::from_str(s).unwrap();
        assert!(config.passthrough_headers.is_empty());
        Ok(())
    }
    #[test]
    fn test_allowed_headers_passthrough_in_config() -> Result<(), Error> {
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
detectors:
    hap:
        type: text_contents
        service:
            hostname: localhost
            port: 9000
            tls: detector
        chunker_id: sentence-fr
        default_threshold: 0.5

passthrough_headers:
        - test-header
        "#;
        let config: OrchestratorConfig = serde_yml::from_str(s).unwrap();
        assert_eq!(
            config.passthrough_headers,
            HashSet::from(["test-header".to_string()])
        );
        Ok(())
    }
}
