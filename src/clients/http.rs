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

use std::ops::Deref;

use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{
    body::{Body, Bytes, Incoming},
    HeaderMap, Method, StatusCode,
};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use serde::{de::DeserializeOwned, Serialize};
use tracing::{debug, error, instrument, Span};
use url::Url;

use super::{Client, Error};
use crate::utils::trace::trace_context_from_http_response;
use crate::{
    health::{HealthCheckResult, HealthStatus, OptionalHealthCheckResponseBody},
    utils::{trace, AsUriExt},
};

/// HTTP response type, thin wrapper for `hyper::http::Response` with extra functionality.
pub struct Response(pub hyper::http::Response<BoxBody<Bytes, hyper::Error>>);

impl Response {
    /// Deserializes the response body as JSON into type `T`.
    pub async fn json<T: DeserializeOwned>(self) -> Result<T, Error> {
        let data = self
            .0
            .into_body()
            .boxed()
            .collect()
            .await
            .expect("unexpected infallible error")
            .to_bytes();
        serde_json::from_slice::<T>(&data)
            .map_err(|e| Error::internal("client response deserialization failed", e))
    }
}

impl Deref for Response {
    type Target = hyper::http::response::Response<BoxBody<Bytes, hyper::Error>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<hyper::http::response::Response<Incoming>> for Response {
    fn from(response: hyper::http::Response<Incoming>) -> Self {
        Response(response.map(|body| body.boxed()))
    }
}

pub type HttpClientInner =
    hyper_util::client::legacy::Client<HttpsConnector<HttpConnector>, BoxBody<Bytes, hyper::Error>>;

/// A trait implemented by all clients that use HTTP for their inner client.
pub trait HttpClientExt: Client {
    fn inner(&self) -> &HttpClient;
}

/// An HTTP client wrapping an inner `hyper` HTTP client providing a higher-level API.
#[derive(Clone)]
pub struct HttpClient {
    base_url: Url,
    health_url: Url,
    inner: HttpClientInner,
}

impl HttpClient {
    pub fn new(base_url: Url, inner: HttpClientInner) -> Self {
        let health_url = base_url.join("health").unwrap();
        Self {
            base_url,
            health_url,
            inner,
        }
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub fn endpoint(&self, path: impl Into<&'static str>) -> Url {
        self.base_url.join(path.into()).unwrap()
    }

    #[instrument(skip_all, fields(url))]
    pub async fn get(
        &self,
        url: Url,
        headers: HeaderMap,
        body: impl Serialize,
    ) -> Result<Response, Error> {
        self.send(url, Method::GET, headers, body).await
    }

    #[instrument(skip_all, fields(url))]
    pub async fn post(
        &self,
        url: Url,
        headers: HeaderMap,
        body: impl Serialize,
    ) -> Result<Response, Error> {
        self.send(url, Method::POST, headers, body).await
    }

    #[instrument(skip_all, fields(url))]
    pub async fn send(
        &self,
        url: Url,
        method: Method,
        headers: HeaderMap,
        body: impl Serialize,
    ) -> Result<Response, Error> {
        let ctx = opentelemetry::Context::current();
        let headers = trace::with_traceparent_header(&ctx, headers);
        let mut builder = hyper::http::request::Builder::new()
            .method(method)
            .uri(url.as_uri());
        match builder.headers_mut() {
            Some(headers_mut) => {
                headers_mut.extend(headers.clone());
                debug!(
                    ?url,
                    ?headers,
                    // If the body can not be serialized it will be caught below, we don't want to
                    // complicate logging by handling string serialization error here.
                    body = serde_json::to_string(&body).unwrap_or("".to_string()),
                    "sending client request"
                );
                let body =
                    Full::new(Bytes::from(serde_json::to_vec(&body).map_err(|e| {
                        Error::internal("client request serialization failed", e)
                    })?))
                        .map_err(|err| match err {});
                let request = builder
                    .body(body.boxed())
                    .map_err(|e| Error::internal("client request creation failed", e))?;
                let response = self
                    .inner
                    .request(request)
                    .await
                    .map_err(|e| Error::internal("sending client request failed", e))?;
                debug!(
                    status = ?response.status(),
                    headers = ?response.headers(),
                    size = ?response.size_hint(),
                    "incoming client response"
                );
                let span = Span::current();
                trace_context_from_http_response(&span, &response);
                Ok(response.into())
            }
            None => Err(builder.body(body).err().map_or_else(
                || panic!("unexpected request builder error - headers missing in builder but no errors found"),
                |e| Error::internal("client request creation failed", e),
            )),
        }
    }

    /// This is sectioned off to allow for testing.
    pub(super) async fn http_response_to_health_check_result(
        res: Result<Response, Error>,
    ) -> HealthCheckResult {
        match res {
            Ok(response) => {
                if response.0.status() == StatusCode::OK {
                    if let Ok(body) = response.json::<OptionalHealthCheckResponseBody>().await {
                        // If the service provided a body, we only anticipate a minimal health status and optional reason.
                        HealthCheckResult {
                            status: body.status.clone(),
                            code: StatusCode::OK,
                            reason: match body.status {
                                HealthStatus::Healthy => None,
                                _ => body.reason,
                            },
                        }
                    } else {
                        // If the service did not provide a body, we assume it is healthy.
                        HealthCheckResult {
                            status: HealthStatus::Healthy,
                            code: StatusCode::OK,
                            reason: None,
                        }
                    }
                } else {
                    HealthCheckResult {
                        // The most we can presume is that 5xx errors are likely indicating service issues, implying the service is unhealthy.
                        // and that 4xx errors are more likely indicating health check failures, i.e. due to configuration/implementation issues.
                        // Regardless we can't be certain, so the reason is also provided.
                        // TODO: We will likely circle back to re-evaluate this logic in the future
                        // when we know more about how the client health results will be used.
                        status: if response.status().as_u16() >= 500
                            && response.status().as_u16() < 600
                        {
                            HealthStatus::Unhealthy
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
                        code: response.status(),
                        reason: response.status().canonical_reason().map(|v| v.to_string()),
                    }
                }
            }
            Err(e) => {
                error!("error checking health: {}", e);
                HealthCheckResult {
                    status: HealthStatus::Unknown,
                    code: e.status_code(),
                    reason: Some(e.to_string()),
                }
            }
        }
    }

    pub async fn health(&self) -> HealthCheckResult {
        let res = self.inner.get(self.health_url.as_uri()).await;
        Self::http_response_to_health_check_result(
            res.map(Into::into)
                .map_err(|e| Error::internal("sending client health request failed", e)),
        )
        .await
    }
}

/// Extracts a base url from a url including path segments.
pub fn extract_base_url(url: &Url) -> Option<Url> {
    let mut url = url.clone();
    match url.path_segments_mut() {
        Ok(mut path) => {
            path.clear();
        }
        Err(_) => {
            return None;
        }
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_base_url() {
        let url =
            Url::parse("https://example-detector.route.example.com/api/v1/text/contents").unwrap();
        let base_url = extract_base_url(&url);
        assert_eq!(
            Some(Url::parse("https://example-detector.route.example.com/").unwrap()),
            base_url
        );
        let health_url = base_url.map(|v| v.join("/health").unwrap());
        assert_eq!(
            Some(Url::parse("https://example-detector.route.example.com/health").unwrap()),
            health_url
        );
    }
}
