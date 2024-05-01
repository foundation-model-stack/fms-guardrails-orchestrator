#![allow(clippy::iter_kv_map)]
pub mod clients;
pub mod config;
pub mod models;
pub mod orchestrator;
mod pb;
pub mod server;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    ClientError(#[from] crate::clients::Error),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error(transparent)]
    YamlError(#[from] serde_yml::Error),
}
