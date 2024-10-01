use std::{collections::HashMap, fmt::Display};

use axum::http::StatusCode;
use serde::{ser::SerializeStruct, Deserialize, Serialize};
use tonic::Code;
use tracing::{error, warn};

use crate::{clients::ClientCode, pb::grpc::health::v1::HealthCheckResponse};

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
        // NOTE: gRPC Health v1 status codes: 0 = UNKNOWN, 1 = SERVING, 2 = NOT_SERVING, 3 = SERVICE_UNKNOWN
        match value.status {
            1 => Self::Healthy,
            2 => Self::Unhealthy,
            _ => Self::Unknown,
        }
    }
}

impl From<StatusCode> for HealthStatus {
    fn from(code: StatusCode) -> Self {
        match code.as_u16() {
            200 => Self::Healthy,
            201..=299 => {
                warn!(
                    "Unexpected HTTP successful health check response status code: {}",
                    code
                );
                Self::Healthy
            }
            503 => Self::Unhealthy,
            500..=502 | 504..=599 => {
                warn!(
                    "Unexpected HTTP server error health check response status code: {}",
                    code
                );
                Self::Unhealthy
            }
            _ => {
                warn!(
                    "Unexpected HTTP client error health check response status code: {}",
                    code
                );
                Self::Unknown
            }
        }
    }
}

/// Holds health check results for all clients.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ClientHealth(HashMap<String, HealthCheckResult>);

impl ClientHealth {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self(HashMap::with_capacity(capacity))
    }

    pub fn healthy(&self) -> bool {
        !self
            .0
            .iter()
            .any(|(_, value)| matches!(value.health_status, HealthStatus::Unhealthy))
    }
}

impl std::ops::Deref for ClientHealth {
    type Target = HashMap<String, HealthCheckResult>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ClientHealth {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Result of a health check request.
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    /// Overall health status of client service.
    /// `HEALTHY`, `UNHEALTHY`, or `UNKNOWN`.
    pub health_status: HealthStatus,
    /// Response code of the latest health check request.
    /// This should be omitted on serialization if the health check was successful (when the response is `HTTP 200 OK` or `gRPC 0 OK`).
    pub response_code: ClientCode,
    /// Optional reason for the health check result status being `UNHEALTHY` or `UNKNOWN`.
    /// May be omitted overall if the health check was successful.
    pub reason: Option<String>,
}

impl HealthCheckResult {
    pub fn reason_from_health_check_response(response: &HealthCheckResponse) -> Option<String> {
        match response.status {
            0 => Some("from gRPC health check serving status: UNKNOWN".to_string()),
            1 => None,
            2 => Some("from gRPC health check serving status: NOT_SERVING".to_string()),
            3 => Some("from gRPC health check serving status: SERVICE_UNKNOWN".to_string()),
            _ => {
                error!(
                    "Unexpected gRPC health check serving status: {}",
                    response.status
                );
                Some(format!(
                    "Unexpected gRPC health check serving status: {}",
                    response.status
                ))
            }
        }
    }
}

impl Serialize for HealthCheckResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.health_status {
            HealthStatus::Healthy => self.health_status.serialize(serializer),
            _ => match &self.reason {
                Some(reason) => {
                    let mut state = serializer.serialize_struct("HealthCheckResult", 3)?;
                    state.serialize_field("health_status", &self.health_status)?;
                    state.serialize_field("response_code", &self.response_code.to_string())?;
                    state.serialize_field("reason", reason)?;
                    state.end()
                }
                None => {
                    let mut state = serializer.serialize_struct("HealthCheckResult", 2)?;
                    state.serialize_field("health_status", &self.health_status)?;
                    state.serialize_field("response_code", &self.response_code.to_string())?;
                    state.end()
                }
            },
        }
    }
}

impl Display for HealthCheckResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.reason {
            Some(reason) => write!(
                f,
                "{} ({})\n\t\t\t{}",
                self.health_status, self.response_code, reason
            ),
            None => write!(f, "{} ({})", self.health_status, self.response_code),
        }
    }
}

impl From<Result<tonic::Response<HealthCheckResponse>, tonic::Status>> for HealthCheckResult {
    fn from(result: Result<tonic::Response<HealthCheckResponse>, tonic::Status>) -> Self {
        match result {
            Ok(response) => {
                let response = response.into_inner();
                Self {
                    health_status: response.into(),
                    response_code: ClientCode::Grpc(Code::Ok),
                    reason: Self::reason_from_health_check_response(&response),
                }
            }
            Err(status) => Self {
                health_status: HealthStatus::Unknown,
                response_code: ClientCode::Grpc(status.code()),
                reason: Some(format!("gRPC health check failed: {}", status)),
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
    pub health_status: HealthStatus,
    /// Optional reason for the health check result status being `UNHEALTHY` or `UNKNOWN`.
    /// May be omitted overall if the health check was successful.
    #[serde(default)]
    pub reason: Option<String>,
}
