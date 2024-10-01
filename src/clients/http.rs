use hyper::StatusCode;
use reqwest::Response;
use tracing::error;
use url::Url;

use super::ClientCode;
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
                            health_status: body.health_status.clone(),
                            response_code: ClientCode::Http(StatusCode::OK),
                            reason: match body.health_status {
                                HealthStatus::Healthy => None,
                                _ => body.reason,
                            },
                        }
                    } else {
                        // If the service did not provide a body, we assume it is healthy.
                        HealthCheckResult {
                            health_status: HealthStatus::Healthy,
                            response_code: ClientCode::Http(StatusCode::OK),
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
                        health_status: if response.status().as_u16() >= 500
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

/// Returns `true` if url is a valid base url.
pub fn is_base_url(url: &str) -> bool {
    if let Ok(url) = Url::parse(url) {
        if let Some(base_url) = extract_base_url(&url) {
            return url == base_url;
        }
    }
    false
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

    #[test]
    fn test_is_base_url() {
        let url = "http://localhost";
        assert!(is_base_url(url));

        let url = "https://example-detector.route.example.com/";
        assert!(is_base_url(url));

        let url = "https://example-detector.route.example.com";
        assert!(is_base_url(url));

        let url = "https://example-detector.route.example.com/api/v1/text/contents";
        assert!(!is_base_url(url));

        let url = "https://example-detector.route.example.com/api/v1/";
        assert!(!is_base_url(url));
    }
}
