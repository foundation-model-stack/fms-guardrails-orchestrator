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

use hyper::StatusCode;
use reqwest::Response;
use tracing::error;
use url::Url;

use crate::health::{HealthCheckResult, HealthStatus, OptionalHealthCheckResponseBody};

#[derive(Clone)]
pub struct HttpClient {
    base_url: Url,
    health_url: Url,
    client: reqwest::Client,
}

impl HttpClient {
    pub fn new(base_url: Url, client: reqwest::Client) -> Self {
        let health_url = base_url.join("health").unwrap();
        Self {
            base_url,
            health_url,
            client,
        }
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
                    code: e.status().unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                    reason: Some(e.to_string()),
                }
            }
        }
    }

    pub async fn health(&self) -> HealthCheckResult {
        let res = self.get(self.health_url.clone()).send().await;
        Self::http_response_to_health_check_result(res).await
    }
}

impl std::ops::Deref for HttpClient {
    type Target = reqwest::Client;

    fn deref(&self) -> &Self::Target {
        &self.client
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
