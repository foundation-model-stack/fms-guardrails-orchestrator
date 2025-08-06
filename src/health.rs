use std::fmt::Display;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{
    clients::errors::grpc_to_http_code,
    pb::grpc::health::v1::{HealthCheckResponse, health_check_response::ServingStatus},
};

/// Health status determined for or returned by a client service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HealthStatus {
    /// The service status is healthy.
    Healthy,
    /// The service status is unhealthy.
    Unhealthy,
    /// The service status is unknown.
    Unknown,
}

impl Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "HEALTHY"),
            HealthStatus::Unhealthy => write!(f, "UNHEALTHY"),
            HealthStatus::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

impl From<HealthCheckResponse> for HealthStatus {
    fn from(value: HealthCheckResponse) -> Self {
        match value.status() {
            ServingStatus::Serving => Self::Healthy,
            ServingStatus::NotServing => Self::Unhealthy,
            ServingStatus::Unknown | ServingStatus::ServiceUnknown => Self::Unknown,
        }
    }
}

impl From<StatusCode> for HealthStatus {
    fn from(code: StatusCode) -> Self {
        match code.as_u16() {
            200..=299 => Self::Healthy,
            500..=599 => Self::Unhealthy,
            _ => Self::Unknown,
        }
    }
}

impl HealthCheckResponse {
    pub fn reason(&self) -> Option<String> {
        let status = self.status();
        match status {
            ServingStatus::Serving => None,
            _ => Some(status.as_str_name().to_string()),
        }
    }
}

/// Result of a health check request.
#[derive(Debug, Clone, Serialize)]
pub struct HealthCheckResult {
    /// Overall health status of client service.
    pub status: HealthStatus,
    /// Response code of the latest health check request.
    #[serde(
        with = "http_serde::status_code",
        skip_serializing_if = "StatusCode::is_success"
    )]
    pub code: StatusCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Display for HealthCheckResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.reason {
            Some(reason) => write!(f, "{} ({})\n\t\t\t{}", self.status, self.code, reason),
            None => write!(f, "{} ({})", self.status, self.code),
        }
    }
}

impl From<Result<tonic::Response<HealthCheckResponse>, tonic::Status>> for HealthCheckResult {
    fn from(result: Result<tonic::Response<HealthCheckResponse>, tonic::Status>) -> Self {
        match result {
            Ok(response) => {
                let response = response.into_inner();
                Self {
                    status: response.into(),
                    code: StatusCode::OK,
                    reason: response.reason(),
                }
            }
            Err(status) => Self {
                status: HealthStatus::Unknown,
                code: grpc_to_http_code(status.code()),
                reason: Some(status.message().to_string()),
            },
        }
    }
}

/// An optional response body that can be interpreted from an HTTP health check response.
/// This is a minimal contract that allows HTTP health requests to opt in to more detailed health check responses than just the status code.
/// If the body omitted, the health check response is considered successful if the status code is `HTTP 200 OK`.
#[derive(Deserialize)]
pub struct OptionalHealthCheckResponseBody {
    /// `HEALTHY`, `UNHEALTHY`, or `UNKNOWN`. Although `HEALTHY` is already implied without a body.
    pub status: HealthStatus,
    /// Optional reason for the health check result status being `UNHEALTHY` or `UNKNOWN`.
    /// May be omitted overall if the health check was successful.
    #[serde(default)]
    pub reason: Option<String>,
}
