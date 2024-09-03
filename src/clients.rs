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

#![allow(dead_code)]
// Import error for adding `source` trait
use std::{collections::HashMap, error::Error as _, fmt::Display, pin::Pin, time::Duration};

use futures::{future::join_all, Stream};
use ginepro::LoadBalancedChannel;
use reqwest::{Response, StatusCode};
use tokio::{fs::File, io::AsyncReadExt};
use tracing::{debug, error};
use url::Url;

use crate::{
    config::{ServiceConfig, Tls},
    health::{HealthCheck, HealthCheckResult, HealthStatus, OptionalHealthCheckResponseBody},
};

pub mod chunker;
pub use chunker::ChunkerClient;

pub mod detector;
pub use detector::DetectorClient;

pub mod generation;
pub use generation::GenerationClient;

pub mod tgis;
pub use tgis::TgisClient;

pub mod nlp;
pub use nlp::NlpClient;

pub const DEFAULT_TGIS_PORT: u16 = 8033;
pub const DEFAULT_CAIKIT_NLP_PORT: u16 = 8085;
pub const DEFAULT_CHUNKER_PORT: u16 = 8085;
pub const DEFAULT_DETECTOR_PORT: u16 = 8080;
pub const COMMON_ROUTER_KEY: &str = "common-router";
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_REQUEST_TIMEOUT_SEC: u64 = 600;

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

/// Client errors.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("{}", .message)]
    Grpc { code: StatusCode, message: String },
    #[error("{}", .message)]
    Http { code: StatusCode, message: String },
    #[error("model not found: {model_id}")]
    ModelNotFound { model_id: String },
}

impl Error {
    /// Returns status code.
    pub fn status_code(&self) -> StatusCode {
        match self {
            // Return equivalent http status code for grpc status code
            Error::Grpc { code, .. } => *code,
            // Return http status code for error responses
            // and 500 for other errors
            Error::Http { code, .. } => *code,
            // Return 404 for model not found
            Error::ModelNotFound { .. } => StatusCode::NOT_FOUND,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        // Log lower level source of error.
        // Examples:
        // 1. client error (Connect) // Cases like connection error, wrong port etc.
        // 2. client error (SendRequest) // Cases like cert issues
        error!(
            "http request failed. Source: {}",
            value.source().unwrap().to_string()
        );
        // Return http status code for error responses
        // and 500 for other errors
        let code = match value.status() {
            Some(code) => code,
            None => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::Http {
            code,
            message: value.to_string(),
        }
    }
}

impl From<tonic::Status> for Error {
    fn from(value: tonic::Status) -> Self {
        use tonic::Code::*;
        // Return equivalent http status code for grpc status code
        let code = match value.code() {
            InvalidArgument => StatusCode::BAD_REQUEST,
            Internal => StatusCode::INTERNAL_SERVER_ERROR,
            NotFound => StatusCode::NOT_FOUND,
            DeadlineExceeded => StatusCode::REQUEST_TIMEOUT,
            Unimplemented => StatusCode::NOT_IMPLEMENTED,
            Unauthenticated => StatusCode::UNAUTHORIZED,
            PermissionDenied => StatusCode::FORBIDDEN,
            Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Ok => StatusCode::OK,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::Grpc {
            code,
            message: value.message().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientCode {
    Http(StatusCode),
    Grpc(tonic::Code),
}

impl Display for ClientCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientCode::Http(code) => write!(f, "HTTP {}", code),
            ClientCode::Grpc(code) => write!(f, "gRPC {:?} {}", code, code),
        }
    }
}

#[derive(Clone)]
pub struct HttpClient {
    base_url: Url,
    client: reqwest::Client,
}

impl HttpClient {
    pub fn new(base_url: Url, client: reqwest::Client) -> Self {
        Self { base_url, client }
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// This is sectioned off to allow for testing.
    pub(super) async fn http_response_to_health_check_result(
        res: Result<Response, reqwest::Error>,
    ) -> HealthCheckResult {
        match res {
            Ok(response) => {
                if response.status() == StatusCode::OK {
                    if let Ok(body) = response.json::<OptionalHealthCheckResponseBody>().await {
                        // If the service provided a body, we only anticipate a minimal health status and optional reason.
                        HealthCheckResult {
                            health_status: body.health_status.clone(),
                            response_code: ClientCode::Http(StatusCode::OK),
                            reason: match body.health_status {
                                HealthStatus::Ready => None,
                                _ => body.reason,
                            },
                        }
                    } else {
                        // If the service did not provide a body, we assume it is ready.
                        HealthCheckResult {
                            health_status: HealthStatus::Ready,
                            response_code: ClientCode::Http(StatusCode::OK),
                            reason: None,
                        }
                    }
                } else {
                    HealthCheckResult {
                        // The most we can presume is that 5xx errors are likely indicating service issues, implying the service is not ready,
                        // and that 4xx errors are more likely indicating health check failures, i.e. due to configuration/implementation issues.
                        // Regardless we can't be certain, so the reason is also provided.
                        health_status: if response.status().as_u16() >= 500
                            && response.status().as_u16() < 600
                        {
                            HealthStatus::NotReady
                        } else if response.status().as_u16() >= 400
                            && response.status().as_u16() < 500
                        {
                            HealthStatus::Unknown
                        } else {
                            error!(
                                "unexpected http health check status code: {}",
                                response.status()
                            );
                            HealthStatus::Unknown
                        },
                        response_code: ClientCode::Http(response.status()),
                        reason: Some(format!(
                            "{}{}",
                            response.error_for_status_ref().unwrap_err(),
                            response
                                .text()
                                .await
                                .map(|s| if s.is_empty() {
                                    "".to_string()
                                } else {
                                    format!(": {}", s)
                                })
                                .unwrap_or("".to_string())
                        )),
                    }
                }
            }
            Err(e) => {
                error!("error checking health: {}", e);
                HealthCheckResult {
                    health_status: HealthStatus::Unknown,
                    response_code: ClientCode::Http(
                        e.status().unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                    ),
                    reason: Some(e.to_string()),
                }
            }
        }
    }
}

impl HealthCheck for HttpClient {
    async fn check(&self) -> HealthCheckResult {
        let url = self.base_url.join("health").unwrap();
        let res = self.get(url).send().await;
        Self::http_response_to_health_check_result(res).await
    }
}

impl std::ops::Deref for HttpClient {
    type Target = reqwest::Client;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

pub async fn create_http_clients(
    default_port: u16,
    config: &[(String, ServiceConfig)],
) -> HashMap<String, HttpClient> {
    let clients = config
        .iter()
        .map(|(name, service_config)| async move {
            let port = service_config.port.unwrap_or(default_port);
            let mut base_url = Url::parse(&service_config.hostname).unwrap();
            base_url.set_port(Some(port)).unwrap();
            let request_timeout = Duration::from_secs(
                service_config
                    .request_timeout
                    .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SEC),
            );
            let mut builder = reqwest::ClientBuilder::new()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .timeout(request_timeout);
            if let Some(Tls::Config(tls_config)) = &service_config.tls {
                let mut cert_buf = Vec::new();
                let cert_path = tls_config.cert_path.as_ref().unwrap().as_path();
                File::open(cert_path)
                    .await
                    .unwrap_or_else(|error| {
                        panic!("error reading cert from {cert_path:?}: {error}")
                    })
                    .read_to_end(&mut cert_buf)
                    .await
                    .unwrap();

                if let Some(key_path) = &tls_config.key_path {
                    File::open(key_path)
                        .await
                        .unwrap_or_else(|error| {
                            panic!("error reading key from {key_path:?}: {error}")
                        })
                        .read_to_end(&mut cert_buf)
                        .await
                        .unwrap();
                }
                let identity = reqwest::Identity::from_pem(&cert_buf).unwrap_or_else(|error| {
                    panic!("error parsing bundled client certificate: {error}")
                });

                builder = builder.use_rustls_tls().identity(identity);

                debug!(?tls_config.insecure);
                builder = builder.danger_accept_invalid_certs(tls_config.insecure.unwrap_or(false));

                if let Some(client_ca_cert_path) = &tls_config.client_ca_cert_path {
                    let ca_cert =
                        tokio::fs::read(client_ca_cert_path)
                            .await
                            .unwrap_or_else(|error| {
                                panic!("error reading cert from {client_ca_cert_path:?}: {error}")
                            });
                    let cacert = reqwest::Certificate::from_pem(&ca_cert)
                        .unwrap_or_else(|error| panic!("error parsing ca cert: {error}"));
                    builder = builder.add_root_certificate(cacert)
                }
            }
            let client = builder
                .build()
                .unwrap_or_else(|error| panic!("error creating http client for {name}: {error}"));
            let client = HttpClient::new(base_url, client);
            (name.clone(), client)
        })
        .collect::<Vec<_>>();
    join_all(clients).await.into_iter().collect()
}

async fn create_grpc_clients<C>(
    default_port: u16,
    config: &[(String, ServiceConfig)],
    new: fn(LoadBalancedChannel) -> C,
) -> HashMap<String, C> {
    let clients = config
        .iter()
        .map(|(name, service_config)| async move {
            let request_timeout = Duration::from_secs(service_config.request_timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT_SEC));
            let mut builder = LoadBalancedChannel::builder((
                service_config.hostname.clone(),
                service_config.port.unwrap_or(default_port),
            ))
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .timeout(request_timeout);

            let client_tls_config = if let Some(Tls::Config(tls_config)) = &service_config.tls {
                let cert_path = tls_config.cert_path.as_ref().unwrap().as_path();
                let key_path = tls_config.key_path.as_ref().unwrap().as_path();
                let cert_pem = tokio::fs::read(cert_path)
                    .await
                    .unwrap_or_else(|error| panic!("error reading cert from {cert_path:?}: {error}"));
                let key_pem = tokio::fs::read(key_path)
                    .await
                    .unwrap_or_else(|error| panic!("error reading key from {key_path:?}: {error}"));
                let identity = tonic::transport::Identity::from_pem(cert_pem, key_pem);
                let mut client_tls_config =
                    tonic::transport::ClientTlsConfig::new().identity(identity).with_native_roots().with_webpki_roots();
                if let Some(client_ca_cert_path) = &tls_config.client_ca_cert_path {
                    let client_ca_cert_pem = tokio::fs::read(client_ca_cert_path)
                        .await
                        .unwrap_or_else(|error| {
                            panic!("error reading client ca cert from {client_ca_cert_path:?}: {error}")
                        });
                    client_tls_config = client_tls_config.ca_certificate(
                        tonic::transport::Certificate::from_pem(client_ca_cert_pem),
                    );
                }
                Some(client_tls_config)
            } else {
                None
            };
            if let Some(client_tls_config) = client_tls_config {
                builder = builder.with_tls(client_tls_config);
            }
            let channel = builder.channel().await.unwrap_or_else(|error| panic!("error creating grpc client for {name}: {error}"));
            (name.clone(), new(channel))
        })
        .collect::<Vec<_>>();
    join_all(clients).await.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pb::grpc::health::v1::{health_check_response::ServingStatus, HealthCheckResponse};
    use hyper::http;

    async fn mock_http_response(
        status: StatusCode,
        body: &str,
    ) -> Result<Response, reqwest::Error> {
        Ok(reqwest::Response::from(
            http::Response::builder()
                .status(status)
                .body(body.to_string())
                .unwrap(),
        ))
    }

    async fn mock_grpc_response(
        health_status: Option<i32>,
        tonic_status: Option<tonic::Status>,
    ) -> Result<tonic::Response<HealthCheckResponse>, tonic::Status> {
        match health_status {
            Some(health_status) => Ok(tonic::Response::new(HealthCheckResponse {
                status: health_status,
            })),
            None => Err(tonic_status
                .expect("tonic_status must be provided for test if health_status is None")),
        }
    }

    #[tokio::test]
    async fn test_http_health_check_responses() {
        // READY responses from HTTP 200 OK with or without reason
        let response = [
            (StatusCode::OK, r#"{}"#),
            (StatusCode::OK, r#"{ "health_status": "READY" }"#),
            (
                StatusCode::OK,
                r#"{ "health_status": "meaningless status" }"#,
            ),
            (
                StatusCode::OK,
                r#"{ "health_status": "READY", "reason": "needless reason" }"#,
            ),
        ];
        for (status, body) in response.iter() {
            let response = mock_http_response(*status, body).await;
            let result = HttpClient::http_response_to_health_check_result(response).await;
            assert_eq!(result.health_status, HealthStatus::Ready);
            assert_eq!(result.response_code, ClientCode::Http(StatusCode::OK));
            assert_eq!(result.reason, None);
            let serialized = serde_json::to_string(&result).unwrap();
            assert_eq!(serialized, r#""READY""#);
        }

        // NOT_READY response from HTTP 200 OK without reason
        let response =
            mock_http_response(StatusCode::OK, r#"{ "health_status": "NOT_READY" }"#).await;
        let result = HttpClient::http_response_to_health_check_result(response).await;
        assert_eq!(result.health_status, HealthStatus::NotReady);
        assert_eq!(result.response_code, ClientCode::Http(StatusCode::OK));
        assert_eq!(result.reason, None);
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"NOT_READY","response_code":"HTTP 200 OK"}"#
        );

        // UNKNOWN response from HTTP 200 OK without reason
        let response =
            mock_http_response(StatusCode::OK, r#"{ "health_status": "UNKNOWN" }"#).await;
        let result = HttpClient::http_response_to_health_check_result(response).await;
        assert_eq!(result.health_status, HealthStatus::Unknown);
        assert_eq!(result.response_code, ClientCode::Http(StatusCode::OK));
        assert_eq!(result.reason, None);
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"UNKNOWN","response_code":"HTTP 200 OK"}"#
        );

        // NOT_READY response from HTTP 200 OK with reason
        let response = mock_http_response(
            StatusCode::OK,
            r#"{ "health_status": "NOT_READY", "reason": "some reason" }"#,
        )
        .await;
        let result = HttpClient::http_response_to_health_check_result(response).await;
        assert_eq!(result.health_status, HealthStatus::NotReady);
        assert_eq!(result.response_code, ClientCode::Http(StatusCode::OK));
        assert_eq!(result.reason, Some("some reason".to_string()));
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"NOT_READY","response_code":"HTTP 200 OK","reason":"some reason"}"#
        );

        // UNKNOWN response from HTTP 200 OK with reason
        let response = mock_http_response(
            StatusCode::OK,
            r#"{ "health_status": "UNKNOWN", "reason": "some reason" }"#,
        )
        .await;
        let result = HttpClient::http_response_to_health_check_result(response).await;
        assert_eq!(result.health_status, HealthStatus::Unknown);
        assert_eq!(result.response_code, ClientCode::Http(StatusCode::OK));
        assert_eq!(result.reason, Some("some reason".to_string()));
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"UNKNOWN","response_code":"HTTP 200 OK","reason":"some reason"}"#
        );

        // NOT_READY response from HTTP 503 SERVICE UNAVAILABLE with reason
        let response = mock_http_response(
            StatusCode::SERVICE_UNAVAILABLE,
            r#"{ "message": "some error message" }"#,
        )
        .await;
        let result = HttpClient::http_response_to_health_check_result(response).await;
        assert_eq!(result.health_status, HealthStatus::NotReady);
        assert_eq!(
            result.response_code,
            ClientCode::Http(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert_eq!(result.reason, Some(r#"HTTP status server error (503 Service Unavailable) for url (http://no.url.provided.local/): { "message": "some error message" }"#.to_string()));
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"NOT_READY","response_code":"HTTP 503 Service Unavailable","reason":"HTTP status server error (503 Service Unavailable) for url (http://no.url.provided.local/): { \"message\": \"some error message\" }"}"#
        );

        // UNKNOWN response from HTTP 404 NOT FOUND with reason
        let response = mock_http_response(
            StatusCode::NOT_FOUND,
            r#"{ "message": "service not found" }"#,
        )
        .await;
        let result = HttpClient::http_response_to_health_check_result(response).await;
        assert_eq!(result.health_status, HealthStatus::Unknown);
        assert_eq!(
            result.response_code,
            ClientCode::Http(StatusCode::NOT_FOUND)
        );
        assert_eq!(result.reason, Some(r#"HTTP status client error (404 Not Found) for url (http://no.url.provided.local/): { "message": "service not found" }"#.to_string()));
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"UNKNOWN","response_code":"HTTP 404 Not Found","reason":"HTTP status client error (404 Not Found) for url (http://no.url.provided.local/): { \"message\": \"service not found\" }"}"#
        );

        // NOT_READY response from HTTP 500 INTERNAL SERVER ERROR without reason
        let response = mock_http_response(StatusCode::INTERNAL_SERVER_ERROR, r#""#).await;
        let result = HttpClient::http_response_to_health_check_result(response).await;
        assert_eq!(result.health_status, HealthStatus::NotReady);
        assert_eq!(
            result.response_code,
            ClientCode::Http(StatusCode::INTERNAL_SERVER_ERROR)
        );
        assert_eq!(result.reason, Some("HTTP status server error (500 Internal Server Error) for url (http://no.url.provided.local/)".to_string()));
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"NOT_READY","response_code":"HTTP 500 Internal Server Error","reason":"HTTP status server error (500 Internal Server Error) for url (http://no.url.provided.local/)"}"#
        );

        // UNKNOWN response from HTTP 400 BAD REQUEST without reason
        let response = mock_http_response(StatusCode::BAD_REQUEST, r#""#).await;
        let result = HttpClient::http_response_to_health_check_result(response).await;
        assert_eq!(result.health_status, HealthStatus::Unknown);
        assert_eq!(
            result.response_code,
            ClientCode::Http(StatusCode::BAD_REQUEST)
        );
        assert_eq!(result.reason, Some("HTTP status client error (400 Bad Request) for url (http://no.url.provided.local/)".to_string()));
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"UNKNOWN","response_code":"HTTP 400 Bad Request","reason":"HTTP status client error (400 Bad Request) for url (http://no.url.provided.local/)"}"#
        );
    }

    #[tokio::test]
    async fn test_grpc_health_check_responses() {
        // READY responses from gRPC 0 OK from serving status 1 SERVING
        let response = mock_grpc_response(Some(ServingStatus::Serving as i32), None).await;
        let result = HealthCheckResult::from(response);
        assert_eq!(result.health_status, HealthStatus::Ready);
        assert_eq!(result.response_code, ClientCode::Grpc(tonic::Code::Ok));
        assert_eq!(result.reason, None);
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(serialized, r#""READY""#);

        // NOT_READY response from gRPC 0 OK form serving status 2 NOT_SERVING
        let response = mock_grpc_response(Some(ServingStatus::NotServing as i32), None).await;
        let result = HealthCheckResult::from(response);
        assert_eq!(result.health_status, HealthStatus::NotReady);
        assert_eq!(result.response_code, ClientCode::Grpc(tonic::Code::Ok));
        assert_eq!(
            result.reason,
            Some(
                "gRPC serving status NOT_SERVING: Service is not ready to serve requests"
                    .to_string()
            )
        );
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"NOT_READY","response_code":"gRPC Ok The operation completed successfully","reason":"gRPC serving status NOT_SERVING: Service is not ready to serve requests"}"#
        );

        // UNKNOWN response from gRPC 0 OK from serving status 0 UNKNOWN
        let response = mock_grpc_response(Some(ServingStatus::Unknown as i32), None).await;
        let result = HealthCheckResult::from(response);
        assert_eq!(result.health_status, HealthStatus::Unknown);
        assert_eq!(result.response_code, ClientCode::Grpc(tonic::Code::Ok));
        assert_eq!(
            result.reason,
            Some(
                "gRPC serving status UNKNOWN: Service's health is unexpectedly unknown".to_string()
            )
        );
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"UNKNOWN","response_code":"gRPC Ok The operation completed successfully","reason":"gRPC serving status UNKNOWN: Service's health is unexpectedly unknown"}"#
        );

        // UNKNOWN response from gRPC 0 OK from serving status 3 SERVICE_UNKNOWN
        let response = mock_grpc_response(Some(ServingStatus::ServiceUnknown as i32), None).await;
        let result = HealthCheckResult::from(response);
        assert_eq!(result.health_status, HealthStatus::Unknown);
        assert_eq!(result.response_code, ClientCode::Grpc(tonic::Code::Ok));
        assert_eq!(
            result.reason,
            Some(
                "gRPC serving status SERVICE_UNKNOWN: Service's heath is currently unknown"
                    .to_string()
            )
        );
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"health_status":"UNKNOWN","response_code":"gRPC Ok The operation completed successfully","reason":"gRPC serving status SERVICE_UNKNOWN: Service's heath is currently unknown"}"#
        );

        // UNKNOWN response from other gRPC error codes (covering main ones)
        let response_codes = [
            tonic::Code::InvalidArgument,
            tonic::Code::Internal,
            tonic::Code::NotFound,
            tonic::Code::Unimplemented,
            tonic::Code::Unauthenticated,
            tonic::Code::PermissionDenied,
            tonic::Code::Unavailable,
        ];
        for code in response_codes.iter() {
            let status = tonic::Status::new(*code, "some error message");
            let response = mock_grpc_response(None, Some(status.clone())).await;
            let result = HealthCheckResult::from(response);
            assert_eq!(result.health_status, HealthStatus::Unknown);
            assert_eq!(result.response_code, ClientCode::Grpc(*code));
            assert_eq!(
                result.reason,
                Some(format!("gRPC health check failed: {}", status.clone()))
            );
            let serialized = serde_json::to_string(&result).unwrap();
            assert_eq!(
                serialized,
                format!(
                    r#"{{"health_status":"UNKNOWN","response_code":"gRPC {:?} {}","reason":"gRPC health check failed: status: {:?}, message: \"some error message\", details: [], metadata: MetadataMap {{ headers: {{}} }}"}}"#,
                    code, code, code
                )
            );
        }
    }
}
