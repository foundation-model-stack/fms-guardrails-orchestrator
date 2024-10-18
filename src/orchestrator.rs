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
pub mod streaming;
pub mod unary;

use std::{collections::HashMap, sync::Arc};

use axum::http::header::HeaderMap;
use tokio::{sync::RwLock, time::Instant};
use tracing::{debug, info};
use uuid::Uuid;

use crate::{
    clients::{
        self,
        chunker::ChunkerClient,
        detector::{
            text_context_doc::ContextType, TextChatDetectorClient, TextContextDocDetectorClient,
            TextGenerationDetectorClient,
        },
        openai::OpenAiClient,
        ClientMap, GenerationClient, NlpClient, TextContentsDetectorClient, TgisClient,
    },
    config::{DetectorType, GenerationProvider, OrchestratorConfig},
    health::HealthCheckCache,
    models::{
        ChatDetectionHttpRequest, ContextDocsHttpRequest, DetectionOnGeneratedHttpRequest,
        DetectorParams, GenerationWithDetectionHttpRequest, GuardrailsConfig,
        GuardrailsHttpRequest, GuardrailsTextGenerationParameters, TextContentDetectionHttpRequest,
    },
};

const UNSUITABLE_INPUT_MESSAGE: &str = "Unsuitable input detected. \
    Please check the detected entities on your input and try again \
    with the unsuitable input removed.";

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
    client_health: Arc<RwLock<HealthCheckCache>>,
}

impl Orchestrator {
    pub async fn new(
        config: OrchestratorConfig,
        start_up_health_check: bool,
    ) -> Result<Self, Error> {
        let clients = create_clients(&config).await;
        let ctx = Arc::new(Context { config, clients });
        let orchestrator = Self {
            ctx,
            client_health: Arc::new(RwLock::new(HealthCheckCache::default())),
        };
        debug!("running start up checks");
        orchestrator.on_start_up(start_up_health_check).await?;
        debug!("start up checks completed");
        Ok(orchestrator)
    }

    pub fn config(&self) -> &OrchestratorConfig {
        &self.ctx.config
    }

    /// Perform any start-up actions required by the orchestrator.
    /// This should only error when the orchestrator is unable to start up.
    /// Currently only performs client health probing to have results loaded into the cache.
    pub async fn on_start_up(&self, health_check: bool) -> Result<(), Error> {
        info!("Performing start-up actions for orchestrator...");
        if health_check {
            info!("Probing client health...");
            let client_health = self.client_health(true).await;
            // Results of probe do not affect orchestrator start-up.
            info!("Client health:\n{client_health}");
        }
        Ok(())
    }

    /// Returns client health state.
    pub async fn client_health(&self, probe: bool) -> HealthCheckCache {
        let initialized = !self.client_health.read().await.is_empty();
        if probe || !initialized {
            debug!("refreshing health cache");
            let now = Instant::now();
            let mut health = HealthCheckCache::with_capacity(self.ctx.clients.len());
            // TODO: perform health checks concurrently?
            for (key, client) in self.ctx.clients.iter() {
                let result = client.health().await;
                health.insert(key.into(), result);
            }
            let mut client_health = self.client_health.write().await;
            *client_health = health;
            debug!(
                "refreshing health cache completed in {:.2?}ms",
                now.elapsed().as_millis()
            );
        }
        self.client_health.read().await.clone()
    }
}

/// Slices chars between start and end indices.
fn slice_codepoints(text: &str, start: usize, end: usize) -> String {
    let len = end - start;
    text.chars().skip(start).take(len).collect()
}

/// Applies masks to input text, returning (offset, masked_text) pairs.
fn apply_masks(text: String, masks: Option<&[(usize, usize)]>) -> Vec<(usize, String)> {
    match masks {
        None | Some([]) => vec![(0, text)],
        Some(masks) => masks
            .iter()
            .map(|(start, end)| {
                let masked_text = slice_codepoints(&text, *start, *end);
                (*start, masked_text)
            })
            .collect(),
    }
}

fn get_chunker_ids(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
) -> Result<Vec<String>, Error> {
    detectors
        .keys()
        .map(|detector_id| {
            let chunker_id = ctx
                .config
                .get_chunker_id(detector_id)
                .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?;
            Ok::<String, Error>(chunker_id)
        })
        .collect::<Result<Vec<_>, Error>>()
}

async fn create_clients(config: &OrchestratorConfig) -> ClientMap {
    let mut clients = ClientMap::new();

    // Create generation client
    if let Some(generation) = &config.generation {
        match generation.provider {
            GenerationProvider::Tgis => {
                let tgis_client = TgisClient::new(&generation.service).await;
                let generation_client = GenerationClient::tgis(tgis_client);
                clients.insert("generation".to_string(), generation_client);
            }
            GenerationProvider::Nlp => {
                let nlp_client = NlpClient::new(&generation.service).await;
                let generation_client = GenerationClient::nlp(nlp_client);
                clients.insert("generation".to_string(), generation_client);
            }
        }
    }

    // Create chat generation client
    if let Some(chat_generation) = &config.chat_generation {
        let openai_client = OpenAiClient::new(
            &chat_generation.service,
            chat_generation.health_service.as_ref(),
        )
        .await;
        clients.insert("chat_generation".to_string(), openai_client);
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
        match detector.r#type {
            DetectorType::TextContents => {
                clients.insert(
                    detector_id.into(),
                    TextContentsDetectorClient::new(
                        &detector.service,
                        detector.health_service.as_ref(),
                    )
                    .await,
                );
            }
            DetectorType::TextGeneration => {
                clients.insert(
                    detector_id.into(),
                    TextGenerationDetectorClient::new(
                        &detector.service,
                        detector.health_service.as_ref(),
                    )
                    .await,
                );
            }
            DetectorType::TextChat => {
                clients.insert(
                    detector_id.into(),
                    TextChatDetectorClient::new(
                        &detector.service,
                        detector.health_service.as_ref(),
                    )
                    .await,
                );
            }
            DetectorType::TextContextDoc => {
                clients.insert(
                    detector_id.into(),
                    TextContextDocDetectorClient::new(
                        &detector.service,
                        detector.health_service.as_ref(),
                    )
                    .await,
                );
            }
        }
    }
    clients
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub offset: usize,
    pub text: String,
}

#[derive(Debug)]
pub struct ClassificationWithGenTask {
    pub request_id: Uuid,
    pub model_id: String,
    pub inputs: String,
    pub guardrails_config: GuardrailsConfig,
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
    pub headers: HeaderMap,
}

impl ClassificationWithGenTask {
    pub fn new(request_id: Uuid, request: GuardrailsHttpRequest, headers: HeaderMap) -> Self {
        Self {
            request_id,
            model_id: request.model_id,
            inputs: request.inputs,
            guardrails_config: request.guardrail_config.unwrap_or_default(),
            text_gen_parameters: request.text_gen_parameters,
            headers,
        }
    }
}

/// Task for the /api/v2/text/detection/content endpoint
#[derive(Debug)]
pub struct GenerationWithDetectionTask {
    /// Request unique identifier
    pub request_id: Uuid,

    /// Model ID of the LLM
    pub model_id: String,

    /// User prompt to be sent to the LLM
    pub prompt: String,

    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,

    /// LLM Parameters
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,

    // Headermap
    pub headers: HeaderMap,
}

impl GenerationWithDetectionTask {
    pub fn new(
        request_id: Uuid,
        request: GenerationWithDetectionHttpRequest,
        headers: HeaderMap,
    ) -> Self {
        Self {
            request_id,
            model_id: request.model_id,
            prompt: request.prompt,
            detectors: request.detectors,
            text_gen_parameters: request.text_gen_parameters,
            headers,
        }
    }
}

/// Task for the /api/v2/text/detection/content endpoint
#[derive(Debug)]
pub struct TextContentDetectionTask {
    /// Request unique identifier
    pub request_id: Uuid,

    /// Content to run detection on
    pub content: String,

    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,

    // Headermap
    pub headers: HeaderMap,
}

impl TextContentDetectionTask {
    pub fn new(
        request_id: Uuid,
        request: TextContentDetectionHttpRequest,
        headers: HeaderMap,
    ) -> Self {
        Self {
            request_id,
            content: request.content,
            detectors: request.detectors,
            headers,
        }
    }
}

/// Task for the /api/v1/text/task/detection/context endpoint
#[derive(Debug)]
pub struct ContextDocsDetectionTask {
    /// Request unique identifier
    pub request_id: Uuid,

    /// Content to run detection on
    pub content: String,

    /// Context type
    pub context_type: ContextType,

    /// Context
    pub context: Vec<String>,

    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,

    // Headermap
    pub headers: HeaderMap,
}

impl ContextDocsDetectionTask {
    pub fn new(request_id: Uuid, request: ContextDocsHttpRequest, headers: HeaderMap) -> Self {
        Self {
            request_id,
            content: request.content,
            context_type: request.context_type,
            context: request.context,
            detectors: request.detectors,
            headers,
        }
    }
}

/// Task for the /api/v2/text/detection/chat endpoint
#[derive(Debug)]
pub struct ChatDetectionTask {
    /// Request unique identifier
    pub request_id: Uuid,

    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,

    // Messages to run detection on
    pub messages: Vec<clients::openai::Message>,

    // Headermap
    pub headers: HeaderMap,
}

impl ChatDetectionTask {
    pub fn new(request_id: Uuid, request: ChatDetectionHttpRequest, headers: HeaderMap) -> Self {
        Self {
            request_id,
            detectors: request.detectors,
            messages: request.messages,
            headers,
        }
    }
}

/// Task for the /api/v2/text/detection/generated endpoint
#[derive(Debug)]
pub struct DetectionOnGenerationTask {
    /// Request unique identifier
    pub request_id: Uuid,

    /// User prompt to be sent to the LLM
    pub prompt: String,

    /// Text generated by the LLM
    pub generated_text: String,

    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,

    // Headermap
    pub headers: HeaderMap,
}

impl DetectionOnGenerationTask {
    pub fn new(
        request_id: Uuid,
        request: DetectionOnGeneratedHttpRequest,
        headers: HeaderMap,
    ) -> Self {
        Self {
            request_id,
            prompt: request.prompt,
            generated_text: request.generated_text,
            detectors: request.detectors,
            headers,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct StreamingClassificationWithGenTask {
    pub request_id: Uuid,
    pub model_id: String,
    pub inputs: String,
    pub guardrails_config: GuardrailsConfig,
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
    pub headers: HeaderMap,
}

impl StreamingClassificationWithGenTask {
    pub fn new(request_id: Uuid, request: GuardrailsHttpRequest, headers: HeaderMap) -> Self {
        Self {
            request_id,
            model_id: request.model_id,
            inputs: request.inputs,
            guardrails_config: request.guardrail_config.unwrap_or_default(),
            text_gen_parameters: request.text_gen_parameters,
            headers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_masks() {
        let text = "I want this sentence. I don't want this sentence. I want this sentence too.";
        let masks: Vec<(usize, usize)> = vec![(0, 21), (50, 75)];
        let text_with_offsets = apply_masks(text.into(), Some(&masks));
        let expected_text_with_offsets = vec![
            (0, "I want this sentence.".to_string()),
            (50, "I want this sentence too.".to_string()),
        ];
        assert_eq!(text_with_offsets, expected_text_with_offsets)
    }

    #[test]
    fn test_slice_codepoints() {
        let s = "Hello world";
        assert_eq!(slice_codepoints(s, 0, 5), "Hello");
        let s = "哈囉世界";
        assert_eq!(slice_codepoints(s, 3, 4), "界");
    }
}
