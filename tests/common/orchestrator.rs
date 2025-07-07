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

use std::{
    marker::PhantomData,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use eventsource_stream::{EventStream, Eventsource};
use fms_guardrails_orchestr8::{config::OrchestratorConfig, orchestrator::Orchestrator, server};
use futures::{
    Stream, StreamExt,
    stream::{
        BoxStream, {self},
    },
};
use mocktail::server::MockServer;
use rand::{Rng, SeedableRng, rngs::SmallRng};
use rustls::crypto::ring;
use serde::{Serialize, de::DeserializeOwned};
use tracing::{error, warn};
use url::Url;

// Default orchestrator configuration file for integration tests.
pub const ORCHESTRATOR_CONFIG_FILE_PATH: &str = "tests/test_config.yaml";

// Endpoints
pub const ORCHESTRATOR_UNARY_ENDPOINT: &str = "/api/v1/task/classification-with-text-generation";
pub const ORCHESTRATOR_STREAMING_ENDPOINT: &str =
    "/api/v1/task/server-streaming-classification-with-text-generation";
pub const ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT: &str =
    "/api/v2/text/generation-detection";

pub const ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT: &str = "/api/v2/text/detection/content";
pub const ORCHESTRATOR_STREAM_CONTENT_DETECTION_ENDPOINT: &str =
    "/api/v2/text/detection/stream-content";
pub const ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT: &str = "/api/v2/text/detection/generated";
pub const ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT: &str = "/api/v2/text/detection/context";
pub const ORCHESTRATOR_CHAT_DETECTION_ENDPOINT: &str = "/api/v2/text/detection/chat";

pub const ORCHESTRATOR_CHAT_COMPLETIONS_DETECTION_ENDPOINT: &str =
    "/api/v2/chat/completions-detection";
pub const ORCHESTRATOR_COMPLETIONS_DETECTION_ENDPOINT: &str = "/api/v2/text/completions-detection";

// Messages
pub const ORCHESTRATOR_UNSUITABLE_INPUT_MESSAGE: &str = "Unsuitable input detected. Please check the detected entities on your input and try again with the unsuitable input removed.";

pub fn ensure_global_rustls_state() {
    let _ = ring::default_provider().install_default();
}

#[derive(Default)]
pub struct TestOrchestratorServerBuilder<'a> {
    config_path: String,
    port: Option<u16>,
    health_port: Option<u16>,
    generation_server: Option<&'a MockServer>,
    openai_server: Option<&'a MockServer>,
    detector_servers: Option<Vec<&'a MockServer>>,
    chunker_servers: Option<Vec<&'a MockServer>>,
}

impl<'a> TestOrchestratorServerBuilder<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn config_path(mut self, config_path: &str) -> Self {
        self.config_path = config_path.into();
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    pub fn health_port(mut self, port: u16) -> Self {
        self.health_port = Some(port);
        self
    }

    pub fn generation_server(mut self, server: &'a MockServer) -> Self {
        self.generation_server = Some(server);
        self
    }

    pub fn openai_server(mut self, server: &'a MockServer) -> Self {
        self.openai_server = Some(server);
        self
    }

    pub fn detector_servers(mut self, servers: impl IntoIterator<Item = &'a MockServer>) -> Self {
        self.detector_servers = Some(servers.into_iter().collect());
        self
    }

    pub fn chunker_servers(mut self, servers: impl IntoIterator<Item = &'a MockServer>) -> Self {
        self.chunker_servers = Some(servers.into_iter().collect());
        self
    }

    pub async fn build(self) -> Result<TestOrchestratorServer, anyhow::Error> {
        // Set default crypto provider
        ensure_global_rustls_state();

        // Load orchestrator config
        let mut config = OrchestratorConfig::load(self.config_path).await?;

        // Start & configure mock servers
        initialize_generation_server(self.generation_server, &mut config).await?;
        initialize_openai_server(self.openai_server, &mut config).await?;
        initialize_detectors(self.detector_servers.as_deref(), &mut config).await?;
        initialize_chunkers(self.chunker_servers.as_deref(), &mut config).await?;

        // Create & start test orchestrator server
        let server = TestOrchestratorServer::start(config).await?;

        Ok(server)
    }
}

pub struct TestOrchestratorServer {
    base_url: Url,
    health_url: Url,
    client: reqwest::Client,
}

impl TestOrchestratorServer {
    pub fn builder<'a>() -> TestOrchestratorServerBuilder<'a> {
        TestOrchestratorServerBuilder::default()
    }

    /// Starts the orchestrator server.
    pub async fn start(config: OrchestratorConfig) -> Result<Self, anyhow::Error> {
        let mut rng = SmallRng::from_os_rng();
        loop {
            let port = rng.random_range(10000..60000);
            let health_port = rng.random_range(10000..60000);
            let orchestrator = Orchestrator::new(config.clone(), false).await?;
            let http_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port);
            let health_http_addr: SocketAddr =
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), health_port);
            match server::run(http_addr, health_http_addr, None, None, None, orchestrator).await {
                Ok(_) => {
                    // Give the server time to become ready.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    return Ok(Self {
                        base_url: Url::parse(&format!("http://0.0.0.0:{port}")).unwrap(),
                        health_url: Url::parse(&format!("http://0.0.0.0:{health_port}/health"))
                            .unwrap(),
                        client: reqwest::Client::builder().build().unwrap(),
                    });
                }
                Err(error) if error.details().starts_with("io error") => {
                    warn!(%error, "failed to start server, trying again with different ports...");
                    continue;
                }
                Err(error) => {
                    error!(%error, "failed to start server");
                    return Err(error.into());
                }
            };
        }
    }

    pub fn server_url(&self, path: &str) -> Url {
        self.base_url.join(path).unwrap()
    }

    pub fn health_url(&self) -> Url {
        self.health_url.clone()
    }

    pub fn get(&self, path: &str) -> reqwest::RequestBuilder {
        let url = self.server_url(path);
        self.client.get(url)
    }

    pub fn post(&self, path: &str) -> reqwest::RequestBuilder {
        let url = self.server_url(path);
        self.client.post(url)
    }
}

/// Starts and configures generation server.
async fn initialize_generation_server(
    generation_server: Option<&MockServer>,
    config: &mut OrchestratorConfig,
) -> Result<(), anyhow::Error> {
    if let Some(generation_server) = generation_server {
        generation_server.start().await?;
        config.generation.as_mut().unwrap().service.port =
            Some(generation_server.addr().unwrap().port());
    };
    Ok(())
}

/// Starts and configures chat generation server.
async fn initialize_openai_server(
    openai_server: Option<&MockServer>,
    config: &mut OrchestratorConfig,
) -> Result<(), anyhow::Error> {
    if let Some(openai_server) = openai_server {
        openai_server.start().await?;
        config.openai.as_mut().unwrap().service.port = Some(openai_server.addr().unwrap().port());
    };
    Ok(())
}

/// Starts and configures detector servers.
async fn initialize_detectors(
    detector_servers: Option<&[&MockServer]>,
    config: &mut OrchestratorConfig,
) -> Result<(), anyhow::Error> {
    if let Some(detector_servers) = detector_servers {
        for detector_server in detector_servers {
            detector_server.start().await?;
            config
                .detectors
                .get_mut(detector_server.name())
                .unwrap()
                .service
                .port = Some(detector_server.addr().unwrap().port());
        }
    };
    Ok(())
}

/// Starts and configures chunker servers.
async fn initialize_chunkers(
    chunker_servers: Option<&[&MockServer]>,
    config: &mut OrchestratorConfig,
) -> Result<(), anyhow::Error> {
    if let Some(chunker_servers) = chunker_servers {
        for chunker_server in chunker_servers {
            chunker_server.start().await?;
            config
                .chunkers
                .as_mut()
                .unwrap()
                .get_mut(chunker_server.name())
                .unwrap()
                .service
                .port = Some(chunker_server.addr().unwrap().port());
        }
    };
    Ok(())
}

pub struct SseStream<'a, T> {
    stream: EventStream<BoxStream<'static, Result<Bytes, reqwest::Error>>>,
    phantom: PhantomData<&'a T>,
}

impl<T> SseStream<'_, T> {
    pub fn new(stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static) -> Self {
        let stream = stream.boxed().eventsource();
        Self {
            stream,
            phantom: PhantomData,
        }
    }
}

impl<T> Stream for SseStream<'_, T>
where
    T: DeserializeOwned,
{
    type Item = Result<T, server::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.get_mut().stream).poll_next(cx) {
            Poll::Ready(Some(Ok(event))) => {
                if event.data == "[DONE]" {
                    return Poll::Ready(None);
                }
                if event.event == "error" {
                    let error: server::Error = serde_json::from_str(&event.data).unwrap();
                    return Poll::Ready(Some(Err(error)));
                }
                match serde_json::from_str::<T>(&event.data) {
                    Ok(msg) => Poll::Ready(Some(Ok(msg))),
                    Err(error) => {
                        let error = server::Error {
                            code: http::StatusCode::INTERNAL_SERVER_ERROR,
                            details: format!(
                                "sse_stream error: `event.data` deserialization failed {error}"
                            ),
                        };
                        Poll::Ready(Some(Err(error)))
                    }
                }
            }
            Poll::Ready(Some(Err(error))) => {
                let error = server::Error {
                    code: http::StatusCode::INTERNAL_SERVER_ERROR,
                    details: format!("sse_stream error: error parsing event {error}"),
                };
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub fn json_lines_stream(
    messages: impl IntoIterator<Item = impl Serialize>,
) -> impl Stream<Item = Result<Vec<u8>, std::io::Error>> {
    let chunks = messages
        .into_iter()
        .map(|msg| {
            let mut bytes = serde_json::to_vec(&msg).unwrap();
            bytes.push(b'\n');
            Ok(bytes)
        })
        .collect::<Vec<Result<_, std::io::Error>>>();
    stream::iter(chunks)
}
