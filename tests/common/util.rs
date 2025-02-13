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
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use eventsource_stream::{EventStream, Eventsource};
use fms_guardrails_orchestr8::{
    config::OrchestratorConfig, orchestrator::Orchestrator, server::ServerState,
};
use futures::{stream::BoxStream, Stream, StreamExt};
use mocktail::server::HttpMockServer;
use rustls::crypto::ring;
use serde::de::DeserializeOwned;
use tokio::task::JoinHandle;
use url::Url;

use super::{chunker::MockChunkersServiceServer, generation::MockNlpServiceServer};

/// Default orchestrator configuration file for integration tests.
pub const CONFIG_FILE_PATH: &str = "tests/test.config.yaml";

pub fn ensure_global_rustls_state() {
    let _ = ring::default_provider().install_default();
}

/// Starts mock servers and adds them to orchestrator configuration.
pub async fn create_orchestrator_shared_state(
    generation_server: Option<&MockNlpServiceServer>,
    detectors: Vec<HttpMockServer>,
    chunkers: Vec<(&str, MockChunkersServiceServer)>,
) -> Result<Arc<ServerState>, mocktail::Error> {
    let mut config = OrchestratorConfig::load(CONFIG_FILE_PATH).await.unwrap();

    if let Some(generation_server) = generation_server {
        generation_server.start().await?;
        config.generation.as_mut().unwrap().service.port = Some(generation_server.addr().port());
    }

    for detector_mock_server in detectors {
        detector_mock_server.start().await?;

        // assign mock server port to detector config
        config
            .detectors
            .get_mut(detector_mock_server.name())
            .unwrap()
            .service
            .port = Some(detector_mock_server.addr().port());
    }

    for (chunker_name, chunker_mock_server) in chunkers {
        chunker_mock_server.start().await?;

        // assign mock server port to chunker config
        config
            .chunkers
            .as_mut()
            .unwrap()
            .get_mut(chunker_name)
            .unwrap()
            .service
            .port = Some(chunker_mock_server.addr().port());
    }

    let orchestrator = Orchestrator::new(config, false).await.unwrap();
    Ok(Arc::new(ServerState::new(orchestrator)))
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
        generation_server: Option<MockNlpServiceServer>,
        chat_generation_server: Option<HttpMockServer>,
        detector_servers: Option<Vec<HttpMockServer>>,
        chunker_servers: Option<Vec<(String, MockChunkersServiceServer)>>,
    ) -> Result<Self, anyhow::Error> {
        // Load orchestrator config
        let mut config = OrchestratorConfig::load(config_path).await?;

        // Start & configure mock servers
        // Generation server
        if let Some(generation_server) = generation_server {
            generation_server.start().await?;
            config.generation.as_mut().unwrap().service.port =
                Some(generation_server.addr().port());
        }
        // Chat generation server
        if let Some(chat_generation_server) = chat_generation_server {
            chat_generation_server.start().await?;
            config.chat_generation.as_mut().unwrap().service.port =
                Some(chat_generation_server.addr().port());
        }
        // Detector servers
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
        }
        // Chunker servers
        if let Some(chunker_servers) = chunker_servers {
            for (name, chunker_server) in chunker_servers {
                chunker_server.start().await?;
                config
                    .chunkers
                    .as_mut()
                    .unwrap()
                    .get_mut(&name)
                    .unwrap()
                    .service
                    .port = Some(chunker_server.addr().port());
            }
        }

        // Run orchestrator server
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
        // Allow the server time to become ready
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
