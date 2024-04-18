// Module for defining structure for detector map config and its utilities

use std::{collections::HashMap, path::Path};
use anyhow::Context;
use futures::future::try_join_all;

use serde::{Deserialize, Serialize};


#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct ServiceAddr {
    pub hostname: String,
    pub port: Option<u16>,
    pub tls_enabled: bool,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
    pub tls_ca_path: Option<String>
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub enum ChunkerType {
    #[serde(rename = "SENTENCE")]
    Sentence,
    #[serde(rename = "ALL")]
    All
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct ChunkerConfig {
    pub r#type: ChunkerType
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct DetectorConfig {
<<<<<<< HEAD
    service_config: ServiceAddr,
    config: HashMap<String, String>, // arbitrary keys and values
    chunker: String // chunker id
=======
    pub config: HashMap<String, String>, // things like endpoint, tls key etc.
    pub chunker: String // chunker id
>>>>>>> f090844 (:construction: Begin orchestration)

}

/*
chunkers:
    sentence-en: # chunker-id
        type: Sentence
    sentence-ja: # chunker-id
        type: Sentence
detectors:
    hap-en:
        service_config:
            endpoint: localhost
            port: 8080
            tls: false
        config:
            foo: bar
        chunker: sentence-en

*/
#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct DetectorMap {
    pub chunkers:  HashMap<String, ChunkerConfig>,
    pub detectors: HashMap<String, DetectorConfig>
}


/*

tgis_config:
    hostname: foo.com
    port: 8080
    tls_enabled: false
detector_config:
    chunkers:
        sentence-en: # chunker-id
            type: Sentence
        sentence-ja: # chunker-id
            type: Sentence
    detectors:
        hap-en:
            service_config:
                endpoint: localhost
                port: 8080
                tls_enabled: false
            config:
                foo: bar
            chunker: sentence-en
*/
#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct OrchestratorConfig {
    pub tgis_config: ServiceAddr,
    pub detector_config: DetectorMap
}

impl OrchestratorConfig {
    pub fn load(path: impl AsRef<Path>) -> Self {
        let s = std::fs::read_to_string(path).expect("Failed to load detector map config");
        // implicit return here
        serde_yml::from_str(&s).expect("Invalid detector map config")
    }
}


