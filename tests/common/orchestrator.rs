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
    path::Path,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use eventsource_stream::{EventStream, Eventsource};
use fms_guardrails_orchestr8::{config::OrchestratorConfig, orchestrator::Orchestrator};
use futures::{stream::BoxStream, Stream, StreamExt};
use mocktail::server::{GrpcMockServer, HttpMockServer};
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
pub const ORCHESTRATOR_DETECTION_ON_GENERATION_ENDPOINT: &str = "/api/v2/text/detection/generated";
pub const ORCHESTRATOR_CONTEXT_DOCS_DETECTION_ENDPOINT: &str = "/api/v2/text/detection/context";
pub const ORCHESTRATOR_CHAT_DETECTION_ENDPOINT: &str = "/api/v2/text/detection/chat";

// Messages
pub const ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE: &str =
    "unexpected error occurred while processing request";

pub fn ensure_global_rustls_state() {
    let _ = ring::default_provider().install_default();
}

pub struct TestOrchestratorServer {
    base_url: Url,
    health_url: Url,
    client: reqwest::Client,
    _handle: JoinHandle<Result<(), anyhow::Error>>,
}

impl TestOrchestratorServer {
    /// Configures and runs an orchestrator server.
    pub async fn run(
        config_path: impl AsRef<Path>,
        port: u16,
        health_port: u16,
        generation_server: Option<GrpcMockServer>,
        chat_generation_server: Option<HttpMockServer>,
        detector_servers: Option<Vec<HttpMockServer>>,
        chunker_servers: Option<Vec<GrpcMockServer>>,
    ) -> Result<Self, anyhow::Error> {
        // Set default crypto provider
        ensure_global_rustls_state();

        // Load orchestrator config
        let mut config = OrchestratorConfig::load(config_path).await?;

        // Start & configure mock servers
        Self::initialize_generation_server(generation_server, &mut config).await?;
        Self::initialize_chat_generation_server(chat_generation_server, &mut config).await?;
        Self::initialize_detectors(detector_servers, &mut config).await?;
        Self::initialize_chunkers(chunker_servers, &mut config).await?;

        // Run orchestrator server and returns it
        Self::initialize_orchestrator_server(port, health_port, config).await
    }

    async fn initialize_generation_server(
        generation_server: Option<GrpcMockServer>,
        config: &mut OrchestratorConfig,
    ) -> Result<(), anyhow::Error> {
        Ok(if let Some(generation_server) = generation_server {
            generation_server.start().await?;
            config.generation.as_mut().unwrap().service.port =
                Some(generation_server.addr().port());
        })
    }

    async fn initialize_chat_generation_server(
        chat_generation_server: Option<HttpMockServer>,
        config: &mut OrchestratorConfig,
    ) -> Result<(), anyhow::Error> {
        Ok(
            if let Some(chat_generation_server) = chat_generation_server {
                chat_generation_server.start().await?;
                config.chat_generation.as_mut().unwrap().service.port =
                    Some(chat_generation_server.addr().port());
            },
        )
    }

    async fn initialize_detectors(
        detector_servers: Option<Vec<HttpMockServer>>,
        config: &mut OrchestratorConfig,
    ) -> Result<(), anyhow::Error> {
        Ok(if let Some(detector_servers) = detector_servers {
            for detector_server in detector_servers {
                detector_server.start().await?;
                config
                    .detectors
                    .get_mut(detector_server.name())
                    .unwrap()
                    .service
                    .port = Some(detector_server.addr().port());
            }
        })
    }

    async fn initialize_chunkers(
        chunker_servers: Option<Vec<GrpcMockServer>>,
        config: &mut OrchestratorConfig,
    ) -> Result<(), anyhow::Error> {
        Ok(if let Some(chunker_servers) = chunker_servers {
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
        })
    }

    async fn initialize_orchestrator_server(
        port: u16,
        health_port: u16,
        config: OrchestratorConfig,
    ) -> Result<Self, anyhow::Error> {
        let orchestrator = Orchestrator::new(config, false).await?;
        let http_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port);
        let health_http_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), health_port);
        let _handle = tokio::spawn(async move {
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
        tokio::time::sleep(Duration::from_millis(10)).await;
        let base_url = Url::parse(&format!("http://0.0.0.0:{port}")).unwrap();
        let health_url = Url::parse(&format!("http://0.0.0.0:{health_port}/health")).unwrap();
        let client = reqwest::Client::builder().build().unwrap();
        Ok(Self {
            base_url,
            health_url,
            client,
            _handle,
        })
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
