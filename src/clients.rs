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
use std::{
    any::TypeId,
    collections::{HashMap, hash_map},
    fmt::Debug,
    pin::Pin,
    time::Duration,
};

use async_trait::async_trait;
use axum::http::{Extensions, HeaderMap};
use futures::Stream;
use ginepro::LoadBalancedChannel;
use hyper_timeout::TimeoutConnector;
use hyper_util::rt::TokioExecutor;
use tonic::{Request, metadata::MetadataMap};
use tower::{ServiceBuilder, timeout::TimeoutLayer};
use tracing::{Span, debug, instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use url::Url;

use crate::{
    config::{ServiceConfig, Tls},
    health::HealthCheckResult,
    utils::{tls, trace::with_traceparent_header},
};

pub mod errors;
pub use errors::Error;

pub mod http;
pub use http::{HttpClient, http_trace_layer};

pub mod chunker;

pub mod detector;
pub use detector::TextContentsDetectorClient;

pub mod tgis;
pub use tgis::TgisClient;

pub mod nlp;
pub use nlp::NlpClient;

pub mod generation;
pub use generation::GenerationClient;

pub mod otel_grpc;
pub use otel_grpc::{OtelGrpcLayer, OtelGrpcService};

pub mod openai;

const DEFAULT_CONNECT_TIMEOUT_SEC: u64 = 60;
const DEFAULT_REQUEST_TIMEOUT_SEC: u64 = 600;
const DEFAULT_GRPC_PROBE_INTERVAL_SEC: u64 = 10;

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

mod private {
    pub struct Seal;
}

#[async_trait]
pub trait Client: Send + Sync + 'static {
    /// Returns the name of the client type.
    fn name(&self) -> &str;

    /// Returns the `TypeId` of the client type. Sealed to prevent overrides.
    fn type_id(&self, _: private::Seal) -> TypeId {
        TypeId::of::<Self>()
    }

    /// Performs a client health check.
    async fn health(&self) -> HealthCheckResult;
}

impl dyn Client {
    pub fn is<T: 'static>(&self) -> bool {
        TypeId::of::<T>() == self.type_id(private::Seal)
    }

    pub fn downcast<T: 'static>(self: Box<Self>) -> Result<Box<T>, Box<Self>> {
        if (*self).is::<T>() {
            let ptr = Box::into_raw(self) as *mut T;
            // SAFETY: guaranteed by `is`
            unsafe { Ok(Box::from_raw(ptr)) }
        } else {
            Err(self)
        }
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        if (*self).is::<T>() {
            let ptr = self as *const dyn Client as *const T;
            // SAFETY: guaranteed by `is`
            unsafe { Some(&*ptr) }
        } else {
            None
        }
    }

    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        if (*self).is::<T>() {
            let ptr = self as *mut dyn Client as *mut T;
            // SAFETY: guaranteed by `is`
            unsafe { Some(&mut *ptr) }
        } else {
            None
        }
    }
}

/// A map containing different types of clients.
#[derive(Default)]
pub struct ClientMap(HashMap<String, Box<dyn Client>>);

impl ClientMap {
    /// Creates an empty `ClientMap`.
    #[inline]
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Inserts a client into the map.
    #[inline]
    pub fn insert<V: Client>(&mut self, key: String, value: V) {
        self.0.insert(key, Box::new(value));
    }

    /// Returns a reference to the client trait object.
    #[inline]
    pub fn get(&self, key: &str) -> Option<&dyn Client> {
        self.0.get(key).map(|v| v.as_ref())
    }

    /// Returns a mutable reference to the client trait object.
    #[inline]
    pub fn get_mut(&mut self, key: &str) -> Option<&mut dyn Client> {
        self.0.get_mut(key).map(|v| v.as_mut())
    }

    /// Downcasts and returns a reference to the concrete client type.
    #[inline]
    pub fn get_as<V: Client>(&self, key: &str) -> Option<&V> {
        self.0.get(key)?.downcast_ref::<V>()
    }

    /// Downcasts and returns a mutable reference to the concrete client type.
    #[inline]
    pub fn get_mut_as<V: Client>(&mut self, key: &str) -> Option<&mut V> {
        self.0.get_mut(key)?.downcast_mut::<V>()
    }

    /// Removes a client from the map.
    #[inline]
    pub fn remove(&mut self, key: &str) -> Option<Box<dyn Client>> {
        self.0.remove(key)
    }

    /// An iterator visiting all key-value pairs in arbitrary order.
    #[inline]
    pub fn iter(&self) -> hash_map::Iter<'_, String, Box<dyn Client>> {
        self.0.iter()
    }

    /// An iterator visiting all keys in arbitrary order.
    #[inline]
    pub fn keys(&self) -> hash_map::Keys<'_, String, Box<dyn Client>> {
        self.0.keys()
    }

    /// An iterator visiting all values in arbitrary order.
    #[inline]
    pub fn values(&self) -> hash_map::Values<'_, String, Box<dyn Client>> {
        self.0.values()
    }

    /// Returns the number of elements in the map.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the map contains no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[instrument(skip_all, fields(hostname = service_config.hostname))]
pub async fn create_http_client(
    default_port: u16,
    service_config: &ServiceConfig,
) -> Result<HttpClient, Error> {
    let port = service_config.port.unwrap_or(default_port);
    let protocol = match service_config.tls {
        Some(_) => "https",
        None => "http",
    };
    let mut base_url = Url::parse(&format!("{}://{}", protocol, &service_config.hostname))
        .unwrap_or_else(|e| panic!("error parsing base url: {}", e));
    base_url
        .set_port(Some(port))
        .unwrap_or_else(|_| panic!("error setting port: {}", port));
    debug!(%base_url, "creating HTTP client");

    let connect_timeout = Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SEC);
    let request_timeout = Duration::from_secs(
        service_config
            .request_timeout
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SEC),
    );

    let https_conn_builder = match &service_config.tls {
        Some(Tls::Config(tls)) => hyper_rustls::HttpsConnectorBuilder::new().with_tls_config(
            tls::build_client_config(tls)
                .await
                .map_err(|e| e.into_client_error())?,
        ),
        Some(_) => panic!("unexpected unresolved TLS in client builder"),
        None => hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls::build_insecure_client_config()),
    };
    let https_conn = https_conn_builder
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();

    let mut timeout_conn = TimeoutConnector::new(https_conn);
    timeout_conn.set_connect_timeout(Some(connect_timeout));

    let client =
        hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(timeout_conn);
    let client = ServiceBuilder::new()
        .layer(http_trace_layer())
        .layer(TimeoutLayer::new(request_timeout))
        .service(client);
    Ok(HttpClient::new(base_url, client))
}

#[instrument(skip_all, fields(hostname = service_config.hostname))]
pub async fn create_grpc_client<C: Debug + Clone>(
    default_port: u16,
    service_config: &ServiceConfig,
    new: fn(OtelGrpcService<LoadBalancedChannel>) -> C,
) -> C {
    let port = service_config.port.unwrap_or(default_port);
    let protocol = match service_config.tls {
        Some(_) => "https",
        None => "http",
    };
    let mut base_url = Url::parse(&format!("{}://{}", protocol, &service_config.hostname)).unwrap();
    base_url.set_port(Some(port)).unwrap();
    debug!(%base_url, "creating gRPC client");
    let connect_timeout = Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SEC);
    let request_timeout = Duration::from_secs(
        service_config
            .request_timeout
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SEC),
    );
    let grpc_dns_probe_interval = Duration::from_secs(
        service_config
            .grpc_dns_probe_interval
            .unwrap_or(DEFAULT_GRPC_PROBE_INTERVAL_SEC),
    );
    let mut builder = LoadBalancedChannel::builder((service_config.hostname.clone(), port))
        .dns_probe_interval(grpc_dns_probe_interval)
        .connect_timeout(connect_timeout)
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
        let mut client_tls_config = tonic::transport::ClientTlsConfig::new()
            .identity(identity)
            .with_native_roots()
            .with_webpki_roots();
        if let Some(client_ca_cert_path) = &tls_config.client_ca_cert_path {
            let client_ca_cert_pem =
                tokio::fs::read(client_ca_cert_path)
                    .await
                    .unwrap_or_else(|error| {
                        panic!("error reading client ca cert from {client_ca_cert_path:?}: {error}")
                    });
            client_tls_config = client_tls_config
                .ca_certificate(tonic::transport::Certificate::from_pem(client_ca_cert_pem));
        }
        Some(client_tls_config)
    } else {
        None
    };
    if let Some(client_tls_config) = client_tls_config {
        builder = builder.with_tls(client_tls_config);
    }
    let channel = builder
        .channel()
        .await
        .unwrap_or_else(|error| panic!("error creating grpc client: {error}"));

    // Adds tower::Service wrapper to allow for enable middleware layers to be added
    let channel = ServiceBuilder::new().layer(OtelGrpcLayer).service(channel);
    new(channel)
}

/// Returns `true` if hostname is valid according to [IETF RFC 1123](https://tools.ietf.org/html/rfc1123).
///
/// Conditions:
/// - It does not start or end with `-` or `.`.
/// - It does not contain any characters outside of the alphanumeric range, except for `-` and `.`.
/// - It is not empty.
/// - It is 253 or fewer characters.
/// - Its labels (characters separated by `.`) are not empty.
/// - Its labels are 63 or fewer characters.
/// - Its labels do not start or end with '-' or '.'.
pub fn is_valid_hostname(hostname: &str) -> bool {
    fn is_valid_char(byte: u8) -> bool {
        byte.is_ascii_lowercase()
            || byte.is_ascii_uppercase()
            || byte.is_ascii_digit()
            || byte == b'-'
            || byte == b'.'
    }
    !(hostname.bytes().any(|byte| !is_valid_char(byte))
        || hostname.split('.').any(|label| {
            label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-')
        })
        || hostname.is_empty()
        || hostname.len() > 253)
}

/// Turns a gRPC client request body of type `T` and header map into a `tonic::Request<T>`.
/// Will also inject the current `traceparent` header into the request based on the current span.
fn grpc_request_with_headers<T>(request: T, headers: HeaderMap) -> Request<T> {
    let ctx = Span::current().context();
    let headers = with_traceparent_header(&ctx, headers);
    let metadata = MetadataMap::from_headers(headers);
    Request::from_parts(metadata, Extensions::new(), request)
}

#[cfg(test)]
mod tests {
    use errors::grpc_to_http_code;

    use super::*;
    use crate::{
        health::{HealthCheckResult, HealthStatus},
        pb::grpc::health::v1::{HealthCheckResponse, health_check_response::ServingStatus},
    };

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
    async fn test_grpc_health_check_responses() {
        // READY responses from gRPC 0 OK from serving status 1 SERVING
        let response = mock_grpc_response(Some(ServingStatus::Serving as i32), None).await;
        let result = HealthCheckResult::from(response);
        assert_eq!(result.status, HealthStatus::Healthy);
        assert_eq!(result.code, grpc_to_http_code(tonic::Code::Ok));
        assert_eq!(result.reason, None);
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(serialized, r#"{"status":"HEALTHY"}"#);

        // NOT_READY response from gRPC 0 OK form serving status 2 NOT_SERVING
        let response = mock_grpc_response(Some(ServingStatus::NotServing as i32), None).await;
        let result = HealthCheckResult::from(response);
        assert_eq!(result.status, HealthStatus::Unhealthy);
        assert_eq!(result.code, grpc_to_http_code(tonic::Code::Ok));
        assert_eq!(result.reason, Some("NOT_SERVING".to_string()));
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"status":"UNHEALTHY","reason":"NOT_SERVING"}"#
        );

        // UNKNOWN response from gRPC 0 OK from serving status 0 UNKNOWN
        let response = mock_grpc_response(Some(ServingStatus::Unknown as i32), None).await;
        let result = HealthCheckResult::from(response);
        assert_eq!(result.status, HealthStatus::Unknown);
        assert_eq!(result.code, grpc_to_http_code(tonic::Code::Ok));
        assert_eq!(result.reason, Some("UNKNOWN".to_string()));
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(serialized, r#"{"status":"UNKNOWN","reason":"UNKNOWN"}"#);

        // UNKNOWN response from gRPC 0 OK from serving status 3 SERVICE_UNKNOWN
        let response = mock_grpc_response(Some(ServingStatus::ServiceUnknown as i32), None).await;
        let result = HealthCheckResult::from(response);
        assert_eq!(result.status, HealthStatus::Unknown);
        assert_eq!(result.code, grpc_to_http_code(tonic::Code::Ok));
        assert_eq!(result.reason, Some("SERVICE_UNKNOWN".to_string()));
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"status":"UNKNOWN","reason":"SERVICE_UNKNOWN"}"#
        );

        // UNKNOWN response from other gRPC error codes (covering main ones)
        let codes = [
            tonic::Code::InvalidArgument,
            tonic::Code::Internal,
            tonic::Code::NotFound,
            tonic::Code::Unimplemented,
            tonic::Code::Unauthenticated,
            tonic::Code::PermissionDenied,
            tonic::Code::Unavailable,
        ];
        for code in codes.into_iter() {
            let status = tonic::Status::new(code, "some error message");
            let code = grpc_to_http_code(code);
            let response = mock_grpc_response(None, Some(status.clone())).await;
            let result = HealthCheckResult::from(response);
            assert_eq!(result.status, HealthStatus::Unknown);
            assert_eq!(result.code, code);
            assert_eq!(result.reason, Some("some error message".to_string()));
            let serialized = serde_json::to_string(&result).unwrap();
            assert_eq!(
                serialized,
                format!(
                    r#"{{"status":"UNKNOWN","code":{},"reason":"some error message"}}"#,
                    code.as_u16()
                ),
            );
        }
    }

    #[test]
    fn test_is_valid_hostname() {
        let valid_hostnames = ["localhost", "example.route.cloud.com", "127.0.0.1"];
        for hostname in valid_hostnames {
            assert!(is_valid_hostname(hostname));
        }
        let invalid_hostnames = [
            "-LoCaLhOST_",
            ".invalid",
            "invalid.ending-.char",
            "@asdf",
            "too-long-of-a-hostnameeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        ];
        for hostname in invalid_hostnames {
            assert!(!is_valid_hostname(hostname));
        }
    }
}
