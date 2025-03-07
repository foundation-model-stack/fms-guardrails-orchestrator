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
    collections::HashMap,
    marker::PhantomData,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use eventsource_stream::{EventStream, Eventsource};
use fms_guardrails_orchestr8::{config::OrchestratorConfig, orchestrator::Orchestrator};
use futures::{stream::BoxStream, Stream, StreamExt};
use mocktail::{
    server::{GrpcMockServer, HttpMockServer, MockServer},
    utils::find_available_port,
};
use rustls::crypto::ring;
use serde::de::DeserializeOwned;
use tokio::task::JoinHandle;
use url::Url;

// Default orchestrator configuration file for integration tests.
pub const ORCHESTRATOR_CONFIG_FILE_PATH: &str = "tests/test_config.yaml";

// Endpoints
pub const ORCHESTRATOR_STREAMING_ENDPOINT: &str =
    "/api/v1/task/server-streaming-classification-with-text-generation";
pub const ORCHESTRATOR_CONTENT_DETECTION_ENDPOINT: &str = "/api/v2/text/detection/content";

// Messages
pub const ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE: &str =
    "unexpected error occurred while processing request";
pub const ORCHESTRATOR_UNSUITABLE_INPUT_MESSAGE: &str = "Unsuitable input detected. Please check the detected entities on your input and try again with the unsuitable input removed.";

pub fn ensure_global_rustls_state() {
    let _ = ring::default_provider().install_default();
}

#[derive(Default)]
pub struct TestOrchestratorServerBuilder {
    config_path: String,
    port: Option<u16>,
    health_port: Option<u16>,
    generation_server: Option<GrpcMockServer>,
    chat_generation_server: Option<HttpMockServer>,
    detector_servers: Option<Vec<HttpMockServer>>,
    chunker_servers: Option<Vec<GrpcMockServer>>,
}

impl TestOrchestratorServerBuilder {
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

    pub fn generation_server(mut self, server: GrpcMockServer) -> Self {
        self.generation_server = Some(server);
        self
    }

    pub fn chat_generation_server(mut self, server: HttpMockServer) -> Self {
        self.chat_generation_server = Some(server);
        self
    }

    pub fn detector_servers(mut self, servers: impl IntoIterator<Item = HttpMockServer>) -> Self {
        self.detector_servers = Some(servers.into_iter().collect());
        self
    }

    pub fn chunker_servers(mut self, servers: impl IntoIterator<Item = GrpcMockServer>) -> Self {
        self.chunker_servers = Some(servers.into_iter().collect());
        self
    }

    pub async fn build(self) -> Result<TestOrchestratorServer, anyhow::Error> {
        // Set default crypto provider
        ensure_global_rustls_state();

        // Load orchestrator config
        let mut config = OrchestratorConfig::load(self.config_path).await?;

        // Start & configure mock servers
        initialize_generation_server(&self.generation_server, &mut config).await?;
        initialize_chat_generation_server(&self.chat_generation_server, &mut config).await?;
        initialize_detectors(&self.detector_servers, &mut config).await?;
        initialize_chunkers(&self.chunker_servers, &mut config).await?;

        // Create & start test orchestrator server
        let port = self.port.unwrap_or_else(|| find_available_port().unwrap());
        let health_port = self
            .health_port
            .unwrap_or_else(|| find_available_port().unwrap());
        let mut server = TestOrchestratorServer::new(
            config,
            port,
            health_port,
            self.chunker_servers,
            self.detector_servers,
            self.generation_server,
            self.chat_generation_server,
        );
        server.start().await?;

        Ok(server)
    }
}

pub struct TestOrchestratorServer {
    config: OrchestratorConfig,
    port: u16,
    health_port: u16,
    base_url: Url,
    health_url: Url,
    client: reqwest::Client,
    _handle: Option<JoinHandle<Result<(), anyhow::Error>>>,
    pub chunkers: HashMap<String, GrpcMockServer>,
    pub detectors: HashMap<String, HttpMockServer>,
    pub generation_server: Option<GrpcMockServer>,
    pub chat_generation_server: Option<HttpMockServer>,
}

impl TestOrchestratorServer {
    pub fn new(
        config: OrchestratorConfig,
        port: u16,
        health_port: u16,
        chunkers: Option<Vec<GrpcMockServer>>,
        detectors: Option<Vec<HttpMockServer>>,
        generation_server: Option<GrpcMockServer>,
        chat_generation_server: Option<HttpMockServer>,
    ) -> Self {
        let base_url = Url::parse(&format!("http://0.0.0.0:{port}")).unwrap();
        let health_url = Url::parse(&format!("http://0.0.0.0:{health_port}/health")).unwrap();
        let client = reqwest::Client::builder().build().unwrap();

        let chunkers = match chunkers {
            Some(chunkers) => chunkers
                .into_iter()
                .map(|chunker| (chunker.name().to_string(), chunker))
                .collect(),
            None => HashMap::new(),
        };

        let detectors = match detectors {
            Some(detectors) => detectors
                .into_iter()
                .map(|detector| (detector.name().to_string(), detector))
                .collect(),
            None => HashMap::new(),
        };

        Self {
            config,
            port,
            health_port,
            base_url,
            health_url,
            client,
            _handle: None,
            chunkers,
            detectors,
            generation_server,
            chat_generation_server,
        }
    }

    pub fn builder() -> TestOrchestratorServerBuilder {
        TestOrchestratorServerBuilder::default()
    }

    /// Starts the orchestrator server.
    pub async fn start(&mut self) -> Result<(), anyhow::Error> {
        let orchestrator = Orchestrator::new(self.config.clone(), false).await?;
        let http_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), self.port);
        let health_http_addr: SocketAddr =
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), self.health_port);
        let handle = tokio::spawn(async move {
            fms_guardrails_orchestr8::server::run(
                http_addr,
                health_http_addr,
                None,
                None,
                None,
                orchestrator,
            )
            .await?;
            Ok::<(), anyhow::Error>(())
        });
        self._handle = Some(handle);

        // Give the server time to become ready.
        tokio::time::sleep(Duration::from_millis(10)).await;

        Ok(())
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
    generation_server: &Option<GrpcMockServer>,
    config: &mut OrchestratorConfig,
) -> Result<(), anyhow::Error> {
    if let Some(generation_server) = generation_server {
        generation_server.start().await?;
        config.generation.as_mut().unwrap().service.port = Some(generation_server.addr().port());
    };
    Ok(())
}

/// Starts and configures chat generation server.
async fn initialize_chat_generation_server(
    chat_generation_server: &Option<HttpMockServer>,
    config: &mut OrchestratorConfig,
) -> Result<(), anyhow::Error> {
    if let Some(chat_generation_server) = chat_generation_server {
        chat_generation_server.start().await?;
        config.chat_generation.as_mut().unwrap().service.port =
            Some(chat_generation_server.addr().port());
    };
    Ok(())
}

/// Starts and configures detector servers.
async fn initialize_detectors(
    detector_servers: &Option<Vec<HttpMockServer>>,
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
                .port = Some(detector_server.addr().port());
        }
    };
    Ok(())
}

/// Starts and configures chunker servers.
async fn initialize_chunkers(
    chunker_servers: &Option<Vec<GrpcMockServer>>,
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
                .port = Some(chunker_server.addr().port());
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
    type Item = Result<T, anyhow::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.get_mut().stream).poll_next(cx) {
            Poll::Ready(Some(Ok(event))) => {
                if event.data == "[DONE]" {
                    return Poll::Ready(None);
                }
                match serde_json::from_str::<T>(&event.data) {
                    Ok(msg) => Poll::Ready(Some(Ok(msg))),
                    Err(error) => Poll::Ready(Some(Err(error.into()))),
                }
            }
            Poll::Ready(Some(Err(error))) => Poll::Ready(Some(Err(error.into()))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
