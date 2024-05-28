pub mod errors;
use std::{collections::HashMap, sync::Arc};

pub use errors::Error;
use futures::{
    future::try_join_all,
    stream::{self, StreamExt},
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::{
    clients::{
        self, detector::DetectorRequest, ChunkerClient, DetectorClient, GenerationClient,
        NlpClient, TgisClient,
    },
    config::{GenerationProvider, OrchestratorConfig},
    models::{
        ClassifiedGeneratedTextResult, ClassifiedGeneratedTextStreamResult, DetectorParams,
        GuardrailsConfig, GuardrailsHttpRequest, GuardrailsTextGenerationParameters, InputWarning,
        InputWarningReason, TextGenTokenClassificationResults, TokenClassificationResult,
    },
    pb::{
        caikit::runtime::{
            chunkers::TokenizationTaskRequest as ChunkersTokenizationTaskRequest,
            nlp::{TextGenerationTaskRequest, TokenizationTaskRequest},
        },
        fmaas::{
            BatchedGenerationRequest, BatchedTokenizeRequest, GenerationRequest, TokenizeRequest,
        },
    },
};

const UNSUITABLE_INPUT_MESSAGE: &str = "Unsuitable input detected. \
    Please check the detected entities on your input and try again \
    with the unsuitable input removed.";

struct Context {
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

    /// Handles unary tasks.
    pub async fn handle_classification_with_gen(
        &self,
        task: ClassificationWithGenTask,
    ) -> Result<ClassifiedGeneratedTextResult, Error> {
        info!(
            request_id = ?task.request_id,
            model_id = %task.model_id,
            config = ?task.guardrails_config,
            "handling unary task"
        );
        let ctx = self.ctx.clone();
        let task_handle = tokio::spawn(async move {
            let masks = task.guardrails_config.input_masks();
            let input_detectors = task.guardrails_config.input_detectors();
            let output_detectors = task.guardrails_config.output_detectors();
            // Do input detections
            let input_detections = if let Some(detectors) = input_detectors {
                let detections =
                    chunk_and_detect(ctx.clone(), detectors, task.inputs.clone(), masks).await?;
                if detections.is_empty() {
                    None
                } else {
                    Some(detections)
                }
            } else {
                None
            };
            debug!(?input_detections);
            if input_detections.is_some() {
                // Detected HAP/PII
                // Do tokenization to get input_token_count
                let (input_token_count, _tokens) =
                    tokenize(ctx.clone(), task.model_id.clone(), task.inputs.clone()).await?;
                // Send result with input detections
                Ok(ClassifiedGeneratedTextResult {
                    input_token_count,
                    token_classification_results: TextGenTokenClassificationResults {
                        input: input_detections,
                        output: None,
                    },
                    warnings: Some(vec![InputWarning {
                        id: Some(InputWarningReason::UnsuitableInput),
                        message: Some(UNSUITABLE_INPUT_MESSAGE.to_string()),
                    }]),
                    ..Default::default()
                })
            } else {
                // No HAP/PII detected
                // Do text generation
                let mut generation_results = generate(
                    ctx.clone(),
                    task.model_id.clone(),
                    task.inputs.clone(),
                    task.text_gen_parameters.clone(),
                )
                .await?;
                debug!(?generation_results);
                // Do output detections
                let output_detections = if let Some(detectors) = output_detectors {
                    let generated_text = generation_results
                        .generated_text
                        .clone()
                        .unwrap_or_default();
                    let detections =
                        chunk_and_detect(ctx.clone(), detectors, generated_text, None).await?;
                    if detections.is_empty() {
                        None
                    } else {
                        Some(detections)
                    }
                } else {
                    None
                };
                debug!(?output_detections);
                if output_detections.is_some() {
                    generation_results.token_classification_results.output = output_detections;
                }
                Ok(generation_results)
            }
        });
        match task_handle.await {
            // Task completed successfully
            Ok(Ok(result)) => Ok(result),
            // Task failed, return error propagated from child task that failed
            Ok(Err(error)) => {
                error!(request_id = ?task.request_id, %error, "unary task failed");
                Err(error)
            }
            // Task cancelled or panicked
            Err(error) => {
                let error = error.into();
                error!(request_id = ?task.request_id, %error, "unary task failed");
                Err(error)
            }
        }
    }

    /// Handles streaming tasks.
    pub async fn handle_streaming_classification_with_gen(
        &self,
        task: StreamingClassificationWithGenTask,
    ) -> ReceiverStream<ClassifiedGeneratedTextStreamResult> {
        info!(
            request_id = ?task.request_id,
            model_id = %task.model_id,
            config = ?task.guardrails_config,
            "handling streaming task"
        );
        todo!()
    }
}

/// Executes chunking and detection steps.
async fn chunk_and_detect(
    ctx: Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    text: String,
    masks: Option<&[(usize, usize)]>,
) -> Result<Vec<TokenClassificationResult>, Error> {
    // Apply masks
    let text_with_offsets = match masks {
        None | Some([]) => vec![(0, text)],
        Some(masks) => apply_masks(&text, masks),
    };
    // Create a list of required chunkers
    let chunker_ids = detectors
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
        .collect::<Result<Vec<_>, Error>>()?;
    // Spawn chunking tasks, returning a map of chunker_id->chunks.
    let chunks = chunk(ctx.clone(), chunker_ids, text_with_offsets).await?;
    // Spawn detection tasks
    let detections = detect(ctx.clone(), detectors, chunks).await?;
    Ok(detections)
}

/// Spawns chunking tasks for each chunker.
async fn chunk(
    ctx: Arc<Context>,
    chunker_ids: Vec<String>,
    text_with_offsets: Vec<(usize, String)>,
) -> Result<HashMap<String, Vec<Chunk>>, Error> {
    let tasks = chunker_ids
        .into_iter()
        .map(|chunker_id| {
            let ctx = ctx.clone();
            let text_with_offsets = text_with_offsets.clone();
            tokio::spawn(async move { handle_chunk_task(ctx, chunker_id, text_with_offsets).await })
        })
        .collect::<Vec<_>>();
    let results = try_join_all(tasks)
        .await?
        .into_iter()
        .collect::<Result<HashMap<_, _>, Error>>()?;
    Ok(results)
}

/// Spawns detection tasks for each detector.
async fn detect(
    ctx: Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    chunks: HashMap<String, Vec<Chunk>>,
) -> Result<Vec<TokenClassificationResult>, Error> {
    let tasks = detectors
        .iter()
        .map(|(detector_id, detector_params)| {
            let ctx = ctx.clone();
            let detector_id = detector_id.clone();
            let mut detector_params = detector_params.clone();
            if let Some(default_threshold) = ctx.config.get_default_threshold(&detector_id) {
                // Use a default threshold if threshold is not provided by the user
                detector_params
                    .entry("threshold".into())
                    .or_insert(default_threshold.into());
            }
            let chunker_id =
                ctx.config
                    .get_chunker_id(&detector_id)
                    .ok_or_else(|| Error::DetectorNotFound {
                        detector_id: detector_id.clone(),
                    })?;
            let chunks = chunks.get(&chunker_id).unwrap().clone();
            Ok(tokio::spawn(async move {
                handle_detection_task(ctx, detector_id, detector_params, chunks).await
            }))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let results = try_join_all(tasks)
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, Error>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Ok(results)
}

/// Sends a buffered, concurrent stream of requests to a chunker service.
async fn handle_chunk_task(
    ctx: Arc<Context>,
    chunker_id: String,
    text_with_offsets: Vec<(usize, String)>,
) -> Result<(String, Vec<Chunk>), Error> {
    let chunks = stream::iter(text_with_offsets)
        .map(|(offset, text)| {
            let ctx = ctx.clone();
            let chunker_id = chunker_id.clone();
            async move {
                let request = ChunkersTokenizationTaskRequest { text };
                debug!(
                    %chunker_id,
                    ?request,
                    "sending chunker request"
                );
                let response = ctx
                    .chunker_client
                    .tokenization_task_predict(&chunker_id, request)
                    .await
                    .map_err(|error| Error::ChunkerRequestFailed {
                        chunker_id: chunker_id.clone(),
                        error,
                    })?;
                debug!(
                    %chunker_id,
                    ?response,
                    "received chunker response"
                );
                let results = response
                    .results
                    .into_iter()
                    .map(|token| Chunk {
                        offset: offset + token.start as usize,
                        text: token.text,
                    })
                    .collect::<Vec<_>>();
                Ok::<Vec<Chunk>, Error>(results)
            }
        })
        .buffer_unordered(5)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, Error>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Ok((chunker_id, chunks))
}

/// Sends a buffered, concurrent stream of requests to a detector service.
async fn handle_detection_task(
    ctx: Arc<Context>,
    detector_id: String,
    detector_params: DetectorParams,
    chunks: Vec<Chunk>,
) -> Result<Vec<TokenClassificationResult>, Error> {
    let detections = stream::iter(chunks)
        .map(|chunk| {
            let ctx = ctx.clone();
            let detector_id = detector_id.clone();
            let detector_params = detector_params.clone();
            async move {
                // NOTE: The detector request is expected to change and not actually
                // take parameters. However, any parameters will be ignored for now
                // ref. https://github.com/foundation-model-stack/fms-guardrails-orchestrator/issues/37
                let request = DetectorRequest::new(chunk.text.clone(), detector_params.clone());
                debug!(
                    %detector_id,
                    ?request,
                    "sending detector request"
                );
                let response = ctx
                    .detector_client
                    .classify(&detector_id, request)
                    .await
                    .map_err(|error| Error::DetectorRequestFailed {
                        detector_id: detector_id.clone(),
                        error,
                    })?;
                debug!(
                    %detector_id,
                    ?response,
                    "received detector response"
                );
                // Filter results based on threshold (if applicable) here
                let results = response
                    .detections
                    .into_iter()
                    .filter_map(|detection| {
                        let mut result: TokenClassificationResult = detection.into();
                        result.start += chunk.offset as u32;
                        result.end += chunk.offset as u32;
                        if let Some(threshold_value) = detector_params.get("threshold") {
                            if let Some(threshold) = threshold_value.as_f64() {
                                if result.score >= threshold {
                                    return Some(result);
                                }
                            }
                        } else {
                            return Some(result);
                        }
                        None
                    })
                    .collect::<Vec<_>>();
                Ok::<Vec<TokenClassificationResult>, Error>(results)
            }
        })
        .buffer_unordered(5)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, Error>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Ok(detections)
}

/// Sends tokenize request to a generation service.
async fn tokenize(
    ctx: Arc<Context>,
    model_id: String,
    text: String,
) -> Result<(u32, Vec<String>), Error> {
    let generation_client = ctx.generation_client.clone();
    match generation_client {
        GenerationClient::Tgis(client) => {
            let request = BatchedTokenizeRequest {
                model_id: model_id.clone(),
                requests: vec![TokenizeRequest { text }],
                return_tokens: false,
                return_offsets: false,
                truncate_input_tokens: 0,
            };
            debug!(
                %model_id,
                provider = "tgis",
                ?request,
                "sending tokenize request"
            );
            let mut response =
                client
                    .tokenize(request)
                    .await
                    .map_err(|error| Error::TokenizeRequestFailed {
                        model_id: model_id.clone(),
                        error,
                    })?;
            debug!(
                %model_id,
                provider = "tgis",
                ?response,
                "received tokenize response"
            );
            let response = response.responses.swap_remove(0);
            Ok((response.token_count, response.tokens))
        }
        GenerationClient::Nlp(client) => {
            let request = TokenizationTaskRequest { text };
            debug!(
                %model_id,
                provider = "nlp",
                ?request,
                "sending tokenize request"
            );
            let response = client
                .tokenization_task_predict(&model_id, request)
                .await
                .map_err(|error| Error::TokenizeRequestFailed {
                    model_id: model_id.clone(),
                    error,
                })?;
            debug!(
                %model_id,
                provider = "nlp",
                ?response,
                "received tokenize response"
            );
            let tokens = response
                .results
                .into_iter()
                .map(|token| token.text)
                .collect::<Vec<_>>();
            Ok((response.token_count as u32, tokens))
        }
    }
}

/// Sends generate request to a generation service.
async fn generate(
    ctx: Arc<Context>,
    model_id: String,
    text: String,
    params: Option<GuardrailsTextGenerationParameters>,
) -> Result<ClassifiedGeneratedTextResult, Error> {
    let generation_client = ctx.generation_client.clone();
    match generation_client {
        GenerationClient::Tgis(client) => {
            let params = params.map(Into::into);
            let request = BatchedGenerationRequest {
                model_id: model_id.clone(),
                prefix_id: None,
                requests: vec![GenerationRequest { text }],
                params,
            };
            debug!(
                %model_id,
                provider = "tgis",
                ?request,
                "sending generate request"
            );
            let mut response =
                client
                    .generate(request)
                    .await
                    .map_err(|error| Error::GenerateRequestFailed {
                        model_id: model_id.clone(),
                        error,
                    })?;
            debug!(
                %model_id,
                provider = "tgis",
                ?response,
                "received generate response"
            );
            let response = response.responses.swap_remove(0);
            Ok(ClassifiedGeneratedTextResult {
                generated_text: Some(response.text.clone()),
                finish_reason: Some(response.stop_reason().into()),
                generated_token_count: Some(response.generated_token_count),
                seed: Some(response.seed as u32),
                input_token_count: response.input_token_count,
                warnings: None,
                tokens: if response.tokens.is_empty() {
                    None
                } else {
                    Some(response.tokens.into_iter().map(Into::into).collect())
                },
                input_tokens: if response.input_tokens.is_empty() {
                    None
                } else {
                    Some(response.input_tokens.into_iter().map(Into::into).collect())
                },
                token_classification_results: TextGenTokenClassificationResults {
                    input: None,
                    output: None,
                },
            })
        }
        GenerationClient::Nlp(client) => {
            let request = if let Some(params) = params {
                TextGenerationTaskRequest {
                    text,
                    max_new_tokens: params.max_new_tokens.map(|v| v as i64),
                    min_new_tokens: params.min_new_tokens.map(|v| v as i64),
                    truncate_input_tokens: params.truncate_input_tokens.map(|v| v as i64),
                    decoding_method: params.decoding_method,
                    top_k: params.top_k.map(|v| v as i64),
                    top_p: params.top_p,
                    typical_p: params.typical_p,
                    temperature: params.temperature,
                    repetition_penalty: params.repetition_penalty,
                    max_time: params.max_time,
                    exponential_decay_length_penalty: params
                        .exponential_decay_length_penalty
                        .map(Into::into),
                    stop_sequences: params.stop_sequences.unwrap_or_default(),
                    seed: params.seed.map(|v| v as u64),
                    preserve_input_text: params.preserve_input_text,
                    input_tokens: params.input_tokens,
                    generated_tokens: params.generated_tokens,
                    token_logprobs: params.token_logprobs,
                    token_ranks: params.token_ranks,
                }
            } else {
                TextGenerationTaskRequest {
                    text,
                    ..Default::default()
                }
            };
            debug!(
                %model_id,
                provider = "nlp",
                ?request,
                "sending generate request"
            );
            let response = client
                .text_generation_task_predict(&model_id, request)
                .await
                .map_err(|error| Error::GenerateRequestFailed {
                    model_id: model_id.clone(),
                    error,
                })?;
            debug!(
                %model_id,
                provider = "nlp",
                ?response,
                "received generate response"
            );
            Ok(ClassifiedGeneratedTextResult {
                generated_text: Some(response.generated_text.clone()),
                finish_reason: Some(response.finish_reason().into()),
                generated_token_count: Some(response.generated_tokens as u32),
                seed: Some(response.seed as u32),
                input_token_count: response.input_token_count as u32,
                warnings: None,
                tokens: if response.tokens.is_empty() {
                    None
                } else {
                    Some(response.tokens.into_iter().map(Into::into).collect())
                },
                input_tokens: if response.input_tokens.is_empty() {
                    None
                } else {
                    Some(response.input_tokens.into_iter().map(Into::into).collect())
                },
                token_classification_results: TextGenTokenClassificationResults {
                    input: None,
                    output: None,
                },
            })
        }
    }
}

/// Applies masks to input text, returning (offset, masked_text) pairs.
fn apply_masks(text: &str, masks: &[(usize, usize)]) -> Vec<(usize, String)> {
    let chars = text.chars().collect::<Vec<_>>();
    masks
        .iter()
        .map(|(start, end)| {
            let masked_text = chars[*start..*end].iter().cloned().collect();
            (*start, masked_text)
        })
        .collect()
}

async fn create_clients(
    config: &OrchestratorConfig,
) -> (GenerationClient, ChunkerClient, DetectorClient) {
    // TODO: create better solution for routers
    let generation_client = match config.generation.provider {
        GenerationProvider::Tgis => {
            let client = TgisClient::new(
                clients::DEFAULT_TGIS_PORT,
                &[("tgis-router".to_string(), config.generation.service.clone())],
            )
            .await;
            GenerationClient::Tgis(client)
        }
        GenerationProvider::Nlp => {
            let client = NlpClient::new(
                clients::DEFAULT_CAIKIT_NLP_PORT,
                &[("tgis-router".to_string(), config.generation.service.clone())],
            )
            .await;
            GenerationClient::Nlp(client)
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
struct Chunk {
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
        let text_with_offsets = apply_masks(text, &masks);
        let expected_text_with_offsets = vec![
            (0, "I want this sentence.".to_string()),
            (50, "I want this sentence too.".to_string()),
        ];
        assert_eq!(text_with_offsets, expected_text_with_offsets)
    }
}
