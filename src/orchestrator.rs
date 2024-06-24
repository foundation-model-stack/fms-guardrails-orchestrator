pub mod errors;
use std::{collections::HashMap, sync::Arc};

pub use errors::Error;
use futures::{
    future::try_join_all,
    stream::{self, StreamExt},
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::{
    clients::{
        self, detector::ContentAnalysisRequest, ChunkerClient, DetectorClient, GenerationClient,
        NlpClient, TgisClient, COMMON_ROUTER_KEY,
    },
    config::{GenerationProvider, OrchestratorConfig},
    models::{
        ClassifiedGeneratedTextResult, ClassifiedGeneratedTextStreamResult, DetectorParams,
        GuardrailsConfig, GuardrailsHttpRequest, GuardrailsTextGenerationParameters, InputWarning,
        InputWarningReason, TextGenTokenClassificationResults, TokenClassificationResult,
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
            let input_text = task.inputs.clone();
            let masks = task.guardrails_config.input_masks();
            let input_detectors = task.guardrails_config.input_detectors();
            // Do input detections
            let input_detections = if let Some(detectors) = input_detectors {
                input_detection_task(&ctx, detectors, input_text, masks).await?
            } else {
                None
            };
            debug!(?input_detections);
            if input_detections.is_some() {
                // Detected HAP/PII
                // Do tokenization to get input_token_count
                let (input_token_count, _tokens) =
                    tokenize(&ctx, task.model_id.clone(), task.inputs.clone()).await?;
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
                    &ctx,
                    task.model_id.clone(),
                    task.inputs.clone(),
                    task.text_gen_parameters.clone(),
                )
                .await?;
                debug!(?generation_results);
                // Do output detections
                let output_detectors = task.guardrails_config.output_detectors();
                let output_detections = if let Some(detectors) = output_detectors {
                    let generated_text = generation_results
                        .generated_text
                        .clone()
                        .unwrap_or_default();
                    output_detection_task(&ctx, detectors, generated_text).await?
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
    ) -> Result<ReceiverStream<ClassifiedGeneratedTextStreamResult>, Error> {
        info!(
            request_id = ?task.request_id,
            model_id = %task.model_id,
            config = ?task.guardrails_config,
            "handling streaming task"
        );
        let ctx = self.ctx.clone();
        let task_handle: tokio::task::JoinHandle<
            Result<ReceiverStream<ClassifiedGeneratedTextStreamResult>, _>,
        > = tokio::spawn(async move {
            // TODO: Do input detections
            // TODO: results will eventually have to be mutable
            let generation_results = ReceiverStream::new(
                generate_stream(
                    &ctx,
                    task.model_id.clone(),
                    task.inputs.clone(),
                    task.text_gen_parameters.clone(),
                )
                .await?,
            );
            debug!(?generation_results);
            // TODO: Do output detections
            Ok(generation_results)
        });
        match task_handle.await {
            // Task completed successfully
            Ok(Ok(result)) => Ok(result),
            // Task failed, return error propagated from child task that failed
            Ok(Err(error)) => {
                error!(request_id = ?task.request_id, %error, "streaming task failed");
                Err(error)
            }
            // Task cancelled or panicked
            Err(error) => {
                let error = error.into();
                error!(request_id = ?task.request_id, %error, "streaming task failed");
                Err(error)
            }
        }
    }
}

/***************************** Task handlers ******************************/

/// Handles input detection task.
async fn input_detection_task(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    input_text: String,
    masks: Option<&[(usize, usize)]>,
) -> Result<Option<Vec<TokenClassificationResult>>, Error> {
    let text_with_offsets = apply_masks(input_text, masks);
    let chunker_ids = get_chunker_ids(ctx, detectors)?;
    let chunks = chunk_task(ctx, chunker_ids, text_with_offsets).await?;
    let detections = detection_task(ctx, detectors, chunks).await?;
    Ok((!detections.is_empty()).then_some(detections))
}

/// Handles output detection task.
async fn output_detection_task(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    generated_text: String,
) -> Result<Option<Vec<TokenClassificationResult>>, Error> {
    let text_with_offsets = apply_masks(generated_text, None);
    let chunker_ids = get_chunker_ids(ctx, detectors)?;
    let chunks = chunk_task(ctx, chunker_ids, text_with_offsets).await?;
    let detections = detection_task(ctx, detectors, chunks).await?;
    Ok((!detections.is_empty()).then_some(detections))
}

/// Handles detection task.
async fn detection_task(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    chunks: HashMap<String, Vec<Chunk>>,
) -> Result<Vec<TokenClassificationResult>, Error> {
    // Spawn tasks for each detector
    let tasks =
        detectors
            .iter()
            .map(|(detector_id, detector_params)| {
                let ctx = ctx.clone();
                let detector_id = detector_id.clone();
                let detector_params = detector_params.clone();
                // Get the detector config
                let detector_config = ctx.config.detectors.get(&detector_id).ok_or_else(|| {
                    Error::DetectorNotFound {
                        detector_id: detector_id.clone(),
                    }
                })?;
                // Get the default threshold to use if threshold is not provided by the user
                let default_threshold = detector_config.default_threshold;
                // Get chunker for detector
                let chunker_id = detector_config.chunker_id.as_str();
                let chunks = chunks.get(chunker_id).unwrap().clone();
                Ok(tokio::spawn(async move {
                    detect(ctx, detector_id, default_threshold, detector_params, chunks).await
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

/// Handles chunk task.
async fn chunk_task(
    ctx: &Arc<Context>,
    chunker_ids: Vec<String>,
    text_with_offsets: Vec<(usize, String)>,
) -> Result<HashMap<String, Vec<Chunk>>, Error> {
    // Spawn tasks for each chunker
    let tasks = chunker_ids
        .into_iter()
        .map(|chunker_id| {
            let ctx = ctx.clone();
            let text_with_offsets = text_with_offsets.clone();
            tokio::spawn(async move { chunk_parallel(&ctx, chunker_id, text_with_offsets).await })
        })
        .collect::<Vec<_>>();
    let results = try_join_all(tasks)
        .await?
        .into_iter()
        .collect::<Result<HashMap<_, _>, Error>>()?;
    Ok(results)
}

/************************** Requests to services **************************/

/// Sends a request to a detector service and applies threshold.
async fn detect(
    ctx: Arc<Context>,
    detector_id: String,
    default_threshold: f64,
    detector_params: DetectorParams,
    chunks: Vec<Chunk>,
) -> Result<Vec<TokenClassificationResult>, Error> {
    let detector_id = detector_id.clone();
    let threshold = detector_params.threshold.unwrap_or(default_threshold);
    let contents = chunks.iter().map(|chunk| chunk.text.clone()).collect();
    let request = ContentAnalysisRequest::new(contents);
    debug!(%detector_id, ?request, "sending detector request");
    let response = ctx
        .detector_client
        .text_contents(&detector_id, request)
        .await
        .map_err(|error| Error::DetectorRequestFailed {
            detector_id: detector_id.clone(),
            error,
        })?;
    debug!(%detector_id, ?response, "received detector response");
    let results = chunks
        .into_iter()
        .zip(response)
        .flat_map(|(chunk, response)| {
            response
                .into_iter()
                .filter_map(|resp| {
                    let mut result: TokenClassificationResult = resp.into();
                    result.word =
                        slice_codepoints(&chunk.text, result.start as usize, result.end as usize);
                    result.start += chunk.offset as u32;
                    result.end += chunk.offset as u32;
                    (result.score >= threshold).then_some(result)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Ok::<Vec<TokenClassificationResult>, Error>(results)
}

/// Sends request to chunker service.
async fn chunk(
    ctx: &Arc<Context>,
    chunker_id: String,
    offset: usize,
    text: String,
) -> Result<Vec<Chunk>, Error> {
    debug!(%chunker_id, "sending chunker request");
    let response = ctx
        .chunker_client
        .tokenization_task_predict(&chunker_id, text)
        .await
        .map_err(|error| Error::ChunkerRequestFailed {
            chunker_id: chunker_id.clone(),
            error,
        })?;
    debug!(%chunker_id, ?response, "received chunker response");
    Ok(response
        .results
        .into_iter()
        .map(|token| Chunk {
            offset: offset + token.start as usize,
            text: token.text,
        })
        .collect::<Vec<_>>())
}

/// Sends parallel requests to a chunker service.
async fn chunk_parallel(
    ctx: &Arc<Context>,
    chunker_id: String,
    text_with_offsets: Vec<(usize, String)>,
) -> Result<(String, Vec<Chunk>), Error> {
    let chunks = stream::iter(text_with_offsets)
        .map(|(offset, text)| {
            let ctx = ctx.clone();
            let chunker_id = chunker_id.clone();
            async move {
                let results = chunk(&ctx, chunker_id, offset, text).await?;
                Ok::<Vec<Chunk>, Error>(results)
            }
        })
        .buffered(5)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, Error>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Ok((chunker_id, chunks))
}

/// Sends tokenize request to a generation service.
async fn tokenize(
    ctx: &Arc<Context>,
    model_id: String,
    text: String,
) -> Result<(u32, Vec<String>), Error> {
    ctx.generation_client
        .tokenize(model_id.clone(), text)
        .await
        .map_err(|error| Error::TokenizeRequestFailed {
            model_id: model_id.clone(),
            error,
        })
}

/// Sends generate request to a generation service.
async fn generate(
    ctx: &Arc<Context>,
    model_id: String,
    text: String,
    params: Option<GuardrailsTextGenerationParameters>,
) -> Result<ClassifiedGeneratedTextResult, Error> {
    ctx.generation_client
        .generate(model_id.clone(), text, params)
        .await
        .map_err(|error| Error::GenerateRequestFailed {
            model_id: model_id.clone(),
            error,
        })
}

/// Sends generate stream request to a generation service.
async fn generate_stream(
    ctx: &Arc<Context>,
    model_id: String,
    text: String,
    params: Option<GuardrailsTextGenerationParameters>,
) -> Result<mpsc::Receiver<ClassifiedGeneratedTextStreamResult>, Error> {
    ctx.generation_client
        .generate_stream(model_id.clone(), text, params)
        .await
        .map_err(|error| Error::GenerateRequestFailed {
            model_id: model_id.clone(),
            error,
        })
}

/************************** Helper functions **************************/

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
            GenerationClient::Tgis(client)
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
    use crate::{
        models::FinishReason,
        pb::fmaas::{
            BatchedGenerationRequest, BatchedGenerationResponse, GenerationRequest,
            GenerationResponse, StopReason,
        },
    };

    async fn get_test_context(
        gen_client: GenerationClient,
        chunker_client: Option<ChunkerClient>,
        detector_client: Option<DetectorClient>,
    ) -> Context {
        let chunker_client = chunker_client.unwrap_or_default();
        let detector_client = detector_client.unwrap_or_default();

        Context {
            generation_client: gen_client,
            chunker_client,
            detector_client,
            config: OrchestratorConfig::default(),
        }
    }

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

    // Test for TGIS generation with default parameter
    #[tokio::test]
    async fn test_tgis_generate_with_default_params() {
        // Initialize a mock object from `TgisClient`
        let mut mock_client = TgisClient::faux();

        let sample_text = String::from("sample text");
        let text_gen_model_id = String::from("test-llm-id-1");

        let generation_response = GenerationResponse {
            text: String::from("sample response worked"),
            stop_reason: StopReason::EosToken.into(),
            stop_sequence: String::from("\n"),
            generated_token_count: 3,
            seed: 7,
            ..Default::default()
        };

        let client_generation_response = BatchedGenerationResponse {
            responses: [generation_response].to_vec(),
        };

        let expected_generate_req_args = BatchedGenerationRequest {
            model_id: text_gen_model_id.clone(),
            prefix_id: None,
            requests: [GenerationRequest {
                text: sample_text.clone(),
            }]
            .to_vec(),
            params: None,
        };

        let expected_generate_response = ClassifiedGeneratedTextResult {
            generated_text: Some(client_generation_response.responses[0].text.clone()),
            finish_reason: Some(FinishReason::EosToken),
            generated_token_count: Some(3),
            seed: Some(7),
            ..Default::default()
        };

        // Construct a behavior for the mock object
        faux::when!(mock_client.generate(expected_generate_req_args))
            .once() // TODO: Add with_args
            .then_return(Ok(client_generation_response));

        let mock_generation_client = GenerationClient::Tgis(mock_client.clone());

        let ctx = Arc::new(get_test_context(mock_generation_client, None, None).await);

        // Test request formulation and response processing is as expected
        assert_eq!(
            generate(&ctx, text_gen_model_id, sample_text, None)
                .await
                .unwrap(),
            expected_generate_response
        );
    }
}
