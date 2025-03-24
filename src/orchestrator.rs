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
use futures::Stream;
pub mod chat_completions_detection;
pub mod common;
pub mod detector_processing;
pub mod streaming;
pub mod streaming_content_detection;
pub mod types;
pub mod unary;

use std::{collections::HashMap, pin::Pin, sync::Arc};

use axum::http::header::HeaderMap;
use opentelemetry::trace::TraceId;
use tokio::{sync::RwLock, time::Instant};
use tracing::{debug, info};

use crate::{
    clients::{
        self, ClientMap, GenerationClient, NlpClient, TextContentsDetectorClient, TgisClient,
        chunker::ChunkerClient,
        detector::{
            TextChatDetectorClient, TextContextDocDetectorClient, TextGenerationDetectorClient,
            text_context_doc::ContextType,
        },
        openai::{ChatCompletionsRequest, OpenAiClient},
    },
    config::{DetectorType, GenerationProvider, OrchestratorConfig},
    health::HealthCheckCache,
    models::{
        ChatDetectionHttpRequest, ContextDocsHttpRequest, DetectionOnGeneratedHttpRequest,
        DetectorParams, GenerationWithDetectionHttpRequest, GuardrailsConfig,
        GuardrailsHttpRequest, GuardrailsTextGenerationParameters,
        StreamingContentDetectionRequest, TextContentDetectionHttpRequest,
    },
};

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
        let clients = create_clients(&config).await?;
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

async fn create_clients(config: &OrchestratorConfig) -> Result<ClientMap, Error> {
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
        .await?;
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
                    .await?,
                );
            }
            DetectorType::TextGeneration => {
                clients.insert(
                    detector_id.into(),
                    TextGenerationDetectorClient::new(
                        &detector.service,
                        detector.health_service.as_ref(),
                    )
                    .await?,
                );
            }
            DetectorType::TextChat => {
                clients.insert(
                    detector_id.into(),
                    TextChatDetectorClient::new(
                        &detector.service,
                        detector.health_service.as_ref(),
                    )
                    .await?,
                );
            }
            DetectorType::TextContextDoc => {
                clients.insert(
                    detector_id.into(),
                    TextContextDocDetectorClient::new(
                        &detector.service,
                        detector.health_service.as_ref(),
                    )
                    .await?,
                );
            }
        }
    }
    Ok(clients)
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub offset: usize,
    pub text: String,
}

#[derive(Debug)]
pub struct ClassificationWithGenTask {
    pub trace_id: TraceId,
    pub model_id: String,
    pub inputs: String,
    pub guardrails_config: GuardrailsConfig,
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
    pub headers: HeaderMap,
}

impl ClassificationWithGenTask {
    pub fn new(trace_id: TraceId, request: GuardrailsHttpRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
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
    /// Unique identifier of request trace
    pub trace_id: TraceId,

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
        trace_id: TraceId,
        request: GenerationWithDetectionHttpRequest,
        headers: HeaderMap,
    ) -> Self {
        Self {
            trace_id,
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
    /// Unique identifier of request trace
    pub trace_id: TraceId,

    /// Content to run detection on
    pub content: String,

    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,

    // Headermap
    pub headers: HeaderMap,
}

impl TextContentDetectionTask {
    pub fn new(
        trace_id: TraceId,
        request: TextContentDetectionHttpRequest,
        headers: HeaderMap,
    ) -> Self {
        Self {
            trace_id,
            content: request.content,
            detectors: request.detectors,
            headers,
        }
    }
}

/// Task for the /api/v1/text/task/detection/context endpoint
#[derive(Debug)]
pub struct ContextDocsDetectionTask {
    /// Unique identifier of request trace
    pub trace_id: TraceId,

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
    pub fn new(trace_id: TraceId, request: ContextDocsHttpRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
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
    pub trace_id: TraceId,

    /// Detectors configuration
    pub detectors: HashMap<String, DetectorParams>,

    // Messages to run detection on
    pub messages: Vec<clients::openai::Message>,

    // Headermap
    pub headers: HeaderMap,
}

impl ChatDetectionTask {
    pub fn new(trace_id: TraceId, request: ChatDetectionHttpRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
            detectors: request.detectors,
            messages: request.messages,
            headers,
        }
    }
}

/// Task for the /api/v2/text/detection/generated endpoint
#[derive(Debug)]
pub struct DetectionOnGenerationTask {
    /// Unique identifier of request trace
    pub trace_id: TraceId,

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
        trace_id: TraceId,
        request: DetectionOnGeneratedHttpRequest,
        headers: HeaderMap,
    ) -> Self {
        Self {
            trace_id,
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
    pub trace_id: TraceId,
    pub model_id: String,
    pub inputs: String,
    pub guardrails_config: GuardrailsConfig,
    pub text_gen_parameters: Option<GuardrailsTextGenerationParameters>,
    pub headers: HeaderMap,
}

impl StreamingClassificationWithGenTask {
    pub fn new(trace_id: TraceId, request: GuardrailsHttpRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
            model_id: request.model_id,
            inputs: request.inputs,
            guardrails_config: request.guardrail_config.unwrap_or_default(),
            text_gen_parameters: request.text_gen_parameters,
            headers,
        }
    }
}

#[derive(Debug)]
pub struct ChatCompletionsDetectionTask {
    /// Unique identifier of request trace
    pub trace_id: TraceId,
    /// Chat completion request
    pub request: ChatCompletionsRequest,
    // Headermap
    pub headers: HeaderMap,
}

impl ChatCompletionsDetectionTask {
    pub fn new(trace_id: TraceId, request: ChatCompletionsRequest, headers: HeaderMap) -> Self {
        Self {
            trace_id,
            request,
            headers,
        }
    }
}

pub struct StreamingContentDetectionTask {
    pub trace_id: TraceId,
    pub headers: HeaderMap,
    pub detectors: HashMap<String, DetectorParams>,
    pub input_stream:
        Pin<Box<dyn Stream<Item = Result<StreamingContentDetectionRequest, Error>> + Send>>,
}

impl StreamingContentDetectionTask {
    pub fn new(
        trace_id: TraceId,
        headers: HeaderMap,
        input_stream: Pin<
            Box<dyn Stream<Item = Result<StreamingContentDetectionRequest, Error>> + Send>,
        >,
    ) -> Self {
        Self {
            trace_id,
            headers,
            detectors: HashMap::default(),
            input_stream,
        }
    }
}
