use std::{collections::HashMap, fmt::Display};

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{
    clients::errors::grpc_to_http_code,
    pb::grpc::health::v1::{health_check_response::ServingStatus, HealthCheckResponse},
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

/// A cache to hold the latest health check results for each client service.
/// Orchestrator has a reference-counted mutex-protected instance of this cache.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HealthCheckCache(HashMap<String, HealthCheckResult>);

impl HealthCheckCache {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self(HashMap::with_capacity(capacity))
    }

    /// Returns `true` if all services are healthy or unknown.
    pub fn healthy(&self) -> bool {
        !self
            .0
            .iter()
            .any(|(_, value)| matches!(value.status, HealthStatus::Unhealthy))
    }
}

impl std::ops::Deref for HealthCheckCache {
    type Target = HashMap<String, HealthCheckResult>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for HealthCheckCache {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Display for HealthCheckCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", serde_json::to_string_pretty(self).unwrap())
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
