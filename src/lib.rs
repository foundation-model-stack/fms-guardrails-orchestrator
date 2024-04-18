// Declare modules
pub mod parse;
pub mod server;
pub mod models;
pub mod detector_map_config;

use serde::{Serialize, Deserialize};

// use utoipa::ToSchema;

#[derive(Serialize)]
pub(crate) struct ErrorResponse {
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
