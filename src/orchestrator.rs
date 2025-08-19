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
pub mod errors;
pub use errors::Error;
pub mod common;
pub mod handlers;
pub mod types;

use std::{collections::HashMap, sync::Arc};

use http::HeaderMap;
use tracing::{info, warn};

use crate::{
    clients::{
        ChunkerClient, ClientMap, DetectorClient, GenerationClient, NlpClient, TgisClient,
        openai::OpenAiClient,
    },
    config::{GenerationProvider, OrchestratorConfig},
    health::{HealthCheckResult, HealthStatus},
};

const DEFAULT_MAX_RETRIES: usize = 3;

#[cfg_attr(test, derive(Default))]
pub struct Context {
    config: OrchestratorConfig,
    clients: ClientMap,
}

impl Context {
    pub fn new(config: OrchestratorConfig, clients: ClientMap) -> Self {
        Self { config, clients }
    }
}

/// Handles orchestrator tasks.
#[cfg_attr(test, derive(Default))]
pub struct Orchestrator {
    ctx: Arc<Context>,
}

impl Orchestrator {
    pub async fn new(
        config: OrchestratorConfig,
        start_up_health_check: bool,
    ) -> Result<Self, Error> {
        let clients = create_clients(&config).await?;
        let ctx = Arc::new(Context { config, clients });
        let orchestrator = Self { ctx };
        orchestrator.on_start_up(start_up_health_check).await?;
        Ok(orchestrator)
    }

    pub fn config(&self) -> &OrchestratorConfig {
        &self.ctx.config
    }

    /// Run start up actions.
    pub async fn on_start_up(&self, health_check: bool) -> Result<(), Error> {
        if health_check {
            info!("running client health checks");
            let client_health = self.client_health(HeaderMap::new()).await;
            let unhealthy = client_health
                .iter()
                .any(|(_, health)| matches!(health.status, HealthStatus::Unhealthy));
            info!(
                "client health:\n{}",
                serde_json::to_string_pretty(&client_health).unwrap()
            );
            if unhealthy {
                warn!("one or more clients is unhealthy")
            }
        }
        Ok(())
    }

    /// Returns health status of all clients.
    pub async fn client_health(&self, headers: HeaderMap) -> HashMap<String, HealthCheckResult> {
        // TODO: fix router lifetime issue to run concurrently
        // stream::iter(self.ctx.clients.iter())
        //     .map(|(key, client)| async move {
        //         (key.to_string(), client.health().await)
        //     })
        //     .buffer_unordered(8)
        //     .collect::<HashMap<_, _>>()
        //     .await
        let mut health = HashMap::with_capacity(self.ctx.clients.len());
        for (key, client) in self.ctx.clients.iter() {
            health.insert(key.into(), client.health(headers.clone()).await);
        }
        health
    }
}

async fn create_clients(config: &OrchestratorConfig) -> Result<ClientMap, Error> {
    let mut clients = ClientMap::new();

    // Create generation client
    if let Some(generation) = &config.generation {
        let retries = generation
            .service
            .max_retries
            .unwrap_or(DEFAULT_MAX_RETRIES);
        match generation.provider {
            GenerationProvider::Tgis => {
                let tgis_client = TgisClient::new(&generation.service).await;
                let generation_client = GenerationClient::tgis(tgis_client, retries);
                clients.insert("generation".to_string(), generation_client);
            }
            GenerationProvider::Nlp => {
                let nlp_client = NlpClient::new(&generation.service).await;
                let generation_client = GenerationClient::nlp(nlp_client, retries);
                clients.insert("generation".to_string(), generation_client);
            }
        }
    }

    // Create chat completions client
    if let Some(openai) = &config.openai {
        let openai_client =
            OpenAiClient::new(&openai.service, openai.health_service.as_ref()).await?;
        clients.insert("openai".to_string(), openai_client);
    }

    // Create chunker clients
    if let Some(chunkers) = &config.chunkers {
        for (chunker_id, chunker) in chunkers {
            let chunker_client = ChunkerClient::new(&chunker.service).await;
            clients.insert(chunker_id.to_string(), chunker_client);
        }
    }

    // Create detector clients
    for (detector_id, detector) in &config.detectors {
        clients.insert(
            detector_id.into(),
            DetectorClient::new(&detector.service, detector.health_service.as_ref()).await?,
        );
    }
    Ok(clients)
}
