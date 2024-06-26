pub mod errors;
pub use errors::Error;
pub mod processors;
pub mod streaming;
pub mod unary;

use std::{collections::HashMap, sync::Arc};

use uuid::Uuid;

use crate::{
    clients::{
        self, ChunkerClient, DetectorClient, GenerationClient, NlpClient, TgisClient,
        COMMON_ROUTER_KEY,
    },
    config::{GenerationProvider, OrchestratorConfig},
    models::{
        DetectorParams, GuardrailsConfig, GuardrailsHttpRequest, GuardrailsTextGenerationParameters,
    },
};

const UNSUITABLE_INPUT_MESSAGE: &str = "Unsuitable input detected. \
    Please check the detected entities on your input and try again \
    with the unsuitable input removed.";

pub struct Context {
    config: OrchestratorConfig,
    generation_client: GenerationClient,
    chunker_client: ChunkerClient,
    detector_client: DetectorClient,
}

/// Handles orchestrator tasks.
pub struct Orchestrator {
    ctx: Arc<Context>,
}

impl Orchestrator {
    pub async fn new(config: OrchestratorConfig) -> Result<Self, Error> {
        let (generation_client, chunker_client, detector_client) = create_clients(&config).await;
        let ctx = Arc::new(Context {
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
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
) -> Result<Vec<String>, Error> {
    detectors
        .keys()
        .map(|detector_id| {
            let chunker_id =
                ctx.config
                    .get_chunker_id(detector_id)
                    .ok_or_else(|| Error::DetectorNotFound {
                        detector_id: detector_id.clone(),
                    })?;
            Ok::<String, Error>(chunker_id)
        })
        .collect::<Result<Vec<_>, Error>>()
}

async fn create_clients(
    config: &OrchestratorConfig,
) -> (GenerationClient, ChunkerClient, DetectorClient) {
    // TODO: create better solution for routers
    let generation_client = match config.generation.provider {
        GenerationProvider::Tgis => {
            let client = TgisClient::new(
                clients::DEFAULT_TGIS_PORT,
                &[(
                    COMMON_ROUTER_KEY.to_string(),
                    config.generation.service.clone(),
                )],
            )
            .await;
            GenerationClient::tgis(client)
        }
        GenerationProvider::Nlp => {
            let client = NlpClient::new(
                clients::DEFAULT_CAIKIT_NLP_PORT,
                &[(
                    COMMON_ROUTER_KEY.to_string(),
                    config.generation.service.clone(),
                )],
            )
            .await;
            GenerationClient::nlp(client)
        }
    };
    // TODO: simplify all of this
    let chunker_config = config
        .chunkers
        .iter()
        .map(|(chunker_id, config)| (chunker_id.clone(), config.service.clone()))
        .collect::<Vec<_>>();
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
