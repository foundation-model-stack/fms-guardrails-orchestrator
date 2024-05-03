#![allow(clippy::iter_kv_map, clippy::enum_variant_names)]

use axum::{http::StatusCode, Json};

mod clients;
mod config;
mod models;
mod orchestrator;
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

// TODO: create better errors and properly convert
impl From<Error> for (StatusCode, Json<String>) {
    fn from(value: Error) -> Self {
        use Error::*;
        match value {
            ClientError(error) => match error {
                clients::Error::ModelNotFound(message) => {
                    (StatusCode::UNPROCESSABLE_ENTITY, Json(message))
                }
                clients::Error::ReqwestError(error) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, Json(error.to_string()))
                }
                clients::Error::TonicError(error) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, Json(error.to_string()))
                }
                clients::Error::IoError(_) => todo!(),
            },
            IoError(_) => todo!(),
            YamlError(_) => todo!(),
        }
    }
}
