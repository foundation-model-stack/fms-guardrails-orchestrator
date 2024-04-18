// Module for defining structure for detector map config and its utilities

use std::{collections::HashMap, path::Path};
use anyhow::Context;
use futures::future::try_join_all;

use serde::{Deserialize, Serialize};


#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub enum ChunkerType {
    #[serde(rename = "SENTENCE")]
    Sentence,
    #[serde(rename = "ALL")]
    All
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct ChunkerConfig {
    r#type: ChunkerType
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct DetectorConfig {
    config: HashMap<String, String>, // things like endpoint, tls key etc.
    chunker: String // chunker id

}

/*
chunkers:
    sentence-en: # chunker-id
        type: Sentence
    sentence-ja: # chunker-id
        type: Sentence
detectors:
    hap-en:
        config:
            endpoint: localhost
            port: 8080
        chunker: sentence-en

*/
#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct DetectorMap {
    chunkers:  HashMap<String, ChunkerConfig>,
    detectors: HashMap<String, DetectorConfig>
}

impl DetectorMap {
    pub fn load(path: impl AsRef<Path>) -> Self {
        let s = std::fs::read_to_string(path).expect("Failed to load detector map config");
        // implicit return here
        serde_yml::from_str(&s).expect("Invalid detector map config")
    }
}


