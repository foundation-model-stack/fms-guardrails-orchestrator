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
pub mod aggregators;
pub mod streaming;
pub mod unary;

use std::ops::Deref;
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

use crate::clients::{Client, ClientKind};
use crate::{
    clients::{
        self, detector::ContextType, ChunkerClient, DetectorClient, GenerationClient, NlpClient,
        TgisClient, COMMON_ROUTER_KEY,
    },
    config::{GenerationProvider, OrchestratorConfig},
    models::{
        ContextDocsHttpRequest, DetectionOnGeneratedHttpRequest, DetectorParams,
        GenerationWithDetectionHttpRequest, GuardrailsConfig, GuardrailsHttpRequest,
        GuardrailsTextGenerationParameters, TextContentDetectionHttpRequest,
    },
};

const UNSUITABLE_INPUT_MESSAGE: &str = "Unsuitable input detected. \
    Please check the detected entities on your input and try again \
    with the unsuitable input removed.";

#[cfg_attr(test, derive(Default))]
pub struct ContextInner {
    config: OrchestratorConfig,
    generation_client: GenerationClient,
    chunker_client: ChunkerClient,
    detector_client: DetectorClient,
}

#[cfg_attr(test, derive(Default))]
pub struct Context(Arc<ContextInner>);

impl Context {
    pub fn new(inner: ContextInner) -> Self {
        Self(Arc::new(inner))
    }

    pub fn with_gen(&self, task: Option<&str>) -> Result<Self, Error> {
        if self.0.generation_client.is_configured() {
            Ok(self.clone())
        } else {
            Err(clients::Error::GenerationNotConfigured {
                task: task.unwrap_or("process with generation").to_string(),
            }
            .into())
        }
    }

    pub fn _client(&self, kind: ClientKind) -> &dyn Client {
        match kind {
            ClientKind::Generation => &self.generation_client,
            ClientKind::Chunker => &self.chunker_client,
            ClientKind::Detector => &self.detector_client,
        }
    }
}

impl Clone for Context {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Deref for Context {
    type Target = Arc<ContextInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Handles orchestrator tasks.
pub struct Orchestrator {
    ctx: Context,
}

impl Orchestrator {
    pub async fn new(config: OrchestratorConfig) -> Result<Self, Error> {
        let (generation_client, chunker_client, detector_client) = create_clients(&config).await;
        let ctx = Context::new(ContextInner {
            config,
            generation_client,
            chunker_client,
            detector_client,
        });
        Ok(Self { ctx })
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
    ctx: Context,
    detectors: &HashMap<String, DetectorParams>,
) -> Result<Vec<String>, Error> {
    detectors
        .keys()
        .map(|detector_id| {
            let chunker_id = ctx.config.get_chunker_id(detector_id).ok_or_else(|| {
                Error::from(clients::Error::DetectorNotFound {
                    id: detector_id.clone(),
                    task: "get chunker ids".to_string(),
                })
            })?;
            Ok::<String, Error>(chunker_id)
        })
        .collect::<Result<Vec<_>, Error>>()
}

async fn create_clients(
    config: &OrchestratorConfig,
) -> (GenerationClient, ChunkerClient, DetectorClient) {
    // TODO: create better solution for routers
    let generation_client = match &config.generation {
        Some(generation) => match generation.provider {
            GenerationProvider::Tgis => {
                let client = TgisClient::new(
                    clients::DEFAULT_TGIS_PORT,
                    &[(COMMON_ROUTER_KEY.to_string(), generation.service.clone())],
                )
                .await;
                GenerationClient::tgis(client)
            }
            GenerationProvider::Nlp => {
                let client = NlpClient::new(
                    clients::DEFAULT_CAIKIT_NLP_PORT,
                    &[(COMMON_ROUTER_KEY.to_string(), generation.service.clone())],
                )
                .await;
                GenerationClient::nlp(client)
            }
        },
        None => GenerationClient::default(),
    };

    // TODO: simplify all of this
    let chunker_config = match config.chunkers {
        Some(ref chunkers) => chunkers
            .iter()
            .map(|(chunker_id, config)| (chunker_id.clone(), config.service.clone()))
            .collect::<Vec<_>>(),
        None => vec![],
    };
    let chunker_client = ChunkerClient::new(clients::DEFAULT_CHUNKER_PORT, &chunker_config).await;

    let detector_config = config
        .detectors
        .iter()
        .map(|(detector_id, config)| (detector_id.clone(), config.service.clone()))
        .collect::<Vec<_>>();
    let detector_client =
        DetectorClient::new(clients::DEFAULT_DETECTOR_PORT, &detector_config).await;

    (generation_client, chunker_client, detector_client)
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
}

impl ClassificationWithGenTask {
    pub fn new(request_id: Uuid, request: GuardrailsHttpRequest) -> Self {
        Self {
            request_id,
            model_id: request.model_id,
            inputs: request.inputs,
            guardrails_config: request.guardrail_config.unwrap_or_default(),
            text_gen_parameters: request.text_gen_parameters,
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
}

impl GenerationWithDetectionTask {
    pub fn new(request_id: Uuid, request: GenerationWithDetectionHttpRequest) -> Self {
        Self {
            request_id,
            model_id: request.model_id,
            prompt: request.prompt,
            detectors: request.detectors,
            text_gen_parameters: request.text_gen_parameters,
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
}

impl TextContentDetectionTask {
    pub fn new(request_id: Uuid, request: TextContentDetectionHttpRequest) -> Self {
        Self {
            request_id,
            content: request.content,
            detectors: request.detectors,
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
}

impl ContextDocsDetectionTask {
    pub fn new(request_id: Uuid, request: ContextDocsHttpRequest) -> Self {
        Self {
            request_id,
            content: request.content,
            context_type: request.context_type,
            context: request.context,
            detectors: request.detectors,
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
}

impl DetectionOnGenerationTask {
    pub fn new(request_id: Uuid, request: DetectionOnGeneratedHttpRequest) -> Self {
        Self {
            request_id,
            prompt: request.prompt,
            generated_text: request.generated_text,
            detectors: request.detectors,
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
}

impl StreamingClassificationWithGenTask {
    pub fn new(request_id: Uuid, request: GuardrailsHttpRequest) -> Self {
        Self {
            request_id,
            model_id: request.model_id,
            inputs: request.inputs,
            guardrails_config: request.guardrail_config.unwrap_or_default(),
            text_gen_parameters: request.text_gen_parameters,
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
