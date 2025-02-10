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

use std::{collections::HashMap, sync::Arc};

use axum::http::HeaderMap;
use futures::{
    future::try_join_all,
    stream::{self, StreamExt},
};
use tracing::{debug, error, info, instrument, Instrument, Span};

use super::{
    apply_masks, get_chunker_ids, ChatDetectionTask, Chunk, ClassificationWithGenTask, Context,
    ContextDocsDetectionTask, DetectionOnGenerationTask, Error, GenerationWithDetectionTask,
    Orchestrator, TextContentDetectionTask,
};
use crate::{
    clients::{
        chunker::{tokenize_whole_doc, ChunkerClient, DEFAULT_CHUNKER_ID},
        detector::{
            ChatDetectionRequest, ContentAnalysisRequest, ContentAnalysisResponse,
            ContextDocsDetectionRequest, ContextType, GenerationDetectionRequest,
            TextChatDetectorClient, TextContentsDetectorClient, TextContextDocDetectorClient,
            TextGenerationDetectorClient,
        },
        openai::Message,
        GenerationClient,
    },
    models::{
        ChatDetectionResult, ClassifiedGeneratedTextResult, ContextDocsResult,
        DetectionOnGenerationResult, DetectionResult, DetectionWarning, DetectionWarningReason,
        DetectorParams, GenerationWithDetectionResult, GuardrailsTextGenerationParameters,
        TextContentDetectionResult, TextGenTokenClassificationResults, TokenClassificationResult,
    },
    orchestrator::UNSUITABLE_INPUT_MESSAGE,
    pb::caikit::runtime::chunkers,
};

const DEFAULT_STREAM_BUFFER_SIZE: usize = 5;

impl Orchestrator {
    /// Handles unary tasks.
    #[instrument(skip_all, fields(trace_id = ?task.trace_id, model_id = task.model_id, headers = ?task.headers))]
    pub async fn handle_classification_with_gen(
        &self,
        task: ClassificationWithGenTask,
    ) -> Result<ClassifiedGeneratedTextResult, Error> {
        let ctx = self.ctx.clone();
        let trace_id = task.trace_id;
        let headers = task.headers;
        info!(config = ?task.guardrails_config, "handling classification with generation task");
        let task_handle = tokio::spawn(
            async move {
                let input_text = task.inputs.clone();
                let masks = task.guardrails_config.input_masks();
                let input_detectors = task.guardrails_config.input_detectors();
                // Do input detections
                let input_detections = match input_detectors {
                    Some(detectors) if !detectors.is_empty() => {
                        input_detection_task(
                            &ctx,
                            detectors,
                            input_text.clone(),
                            masks,
                            headers.clone(),
                        )
                        .await?
                    }
                    _ => None,
                };
                debug!(?input_detections);
                if let Some(mut input_detections) = input_detections {
                    // Detected HAP/PII
                    // Do tokenization to get input_token_count
                    let (input_token_count, _tokens) = tokenize(
                        &ctx,
                        task.model_id.clone(),
                        task.inputs.clone(),
                        headers.clone(),
                    )
                    .await?;
                    // Send result with input detections
                    input_detections.sort_by_key(|r| r.start);
                    Ok(ClassifiedGeneratedTextResult {
                        input_token_count,
                        token_classification_results: TextGenTokenClassificationResults {
                            input: Some(input_detections),
                            output: None,
                        },
                        warnings: Some(vec![DetectionWarning {
                            id: Some(DetectionWarningReason::UnsuitableInput),
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
                        headers.clone(),
                    )
                    .await?;
                    debug!(?generation_results);
                    // Do output detections
                    let output_detectors = task.guardrails_config.output_detectors();
                    let output_detections = match output_detectors {
                        Some(detectors) if !detectors.is_empty() => {
                            let generated_text = generation_results
                                .generated_text
                                .clone()
                                .unwrap_or_default();
                            output_detection_task(&ctx, detectors, generated_text, headers).await?
                        }
                        _ => None,
                    };
                    debug!(?output_detections);
                    if let Some(mut output_detections) = output_detections {
                        output_detections.sort_by_key(|r| r.start);
                        generation_results.token_classification_results.output =
                            Some(output_detections);
                    }
                    Ok(generation_results)
                }
            }
            .instrument(Span::current()),
        );
        match task_handle.await {
            // Task completed successfully
            Ok(Ok(result)) => {
                debug!(%trace_id, ?result, "sending result to client");
                info!(%trace_id, "task completed");
                Ok(result)
            }
            // Task failed, return error propagated from child task that failed
            Ok(Err(error)) => {
                error!(%trace_id, %error, "task failed");
                Err(error)
            }
            // Task cancelled or panicked
            Err(error) => {
                let error = error.into();
                error!(%trace_id, %error, "task failed");
                Err(error)
            }
        }
    }

    /// Handles the given generation task, followed by detections.
    #[instrument(skip_all, fields(trace_id = ?task.trace_id, model_id = task.model_id, headers = ?task.headers))]
    pub async fn handle_generation_with_detection(
        &self,
        task: GenerationWithDetectionTask,
    ) -> Result<GenerationWithDetectionResult, Error> {
        info!(
            detectors = ?task.detectors,
            "handling generation with detection task"
        );
        let ctx = self.ctx.clone();
        let headers = task.headers;
        let task_handle = tokio::spawn(
            async move {
                let generation_results = generate(
                    &ctx,
                    task.model_id.clone(),
                    task.prompt.clone(),
                    task.text_gen_parameters.clone(),
                    headers.clone(),
                )
                .await?;

                // call detection
                let detections = try_join_all(
                    task.detectors
                        .iter()
                        .map(|(detector_id, detector_params)| {
                            let ctx = ctx.clone();
                            let detector_id = detector_id.clone();
                            let detector_params = detector_params.clone();
                            let prompt = task.prompt.clone();
                            let generated_text = generation_results
                                .generated_text
                                .clone()
                                .unwrap_or_default();
                            async {
                                detect_for_generation(
                                    ctx,
                                    detector_id,
                                    detector_params,
                                    prompt,
                                    generated_text,
                                    headers.clone(),
                                )
                                .await
                            }
                        })
                        .collect::<Vec<_>>(),
                )
                .await?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();

                debug!(?generation_results);
                Ok(GenerationWithDetectionResult {
                    generated_text: generation_results.generated_text.unwrap_or_default(),
                    input_token_count: generation_results.input_token_count,
                    detections,
                })
            }
            .instrument(Span::current()),
        );
        match task_handle.await {
            // Task completed successfully
            Ok(Ok(result)) => Ok(result),
            // Task failed, return error propagated from child task that failed
            Ok(Err(error)) => {
                error!(trace_id = ?task.trace_id, %error, "generation with detection unary task failed");
                Err(error)
            }
            // Task cancelled or panicked
            Err(error) => {
                let error = error.into();
                error!(trace_id = ?task.trace_id, %error, "generation with detection unary task failed");
                Err(error)
            }
        }
    }

    /// Handles detection on textual content
    #[instrument(skip_all, fields(trace_id = ?task.trace_id, headers = ?task.headers))]
    pub async fn handle_text_content_detection(
        &self,
        task: TextContentDetectionTask,
    ) -> Result<TextContentDetectionResult, Error> {
        info!("handling text content detection task");

        let ctx = self.ctx.clone();
        let headers = task.headers;

        let task_handle = tokio::spawn(
            async move {
                let content = task.content.clone();
                // No masking applied, so offset change is 0
                let offset: usize = 0;
                let text_with_offsets = [(offset, content)].to_vec();

                let detectors = task.detectors.clone();

                let chunker_ids = get_chunker_ids(&ctx, &detectors)?;
                let chunks = chunk_task(&ctx, chunker_ids, text_with_offsets).await?;

                // Call detectors
                let mut detections = try_join_all(
                    task.detectors
                        .iter()
                        .map(|(detector_id, detector_params)| {
                            let ctx = ctx.clone();
                            let detector_id = detector_id.clone();
                            let detector_params = detector_params.clone();
                            let detector_config =
                                ctx.config.detectors.get(&detector_id).unwrap_or_else(|| {
                                    panic!("detector config not found for {}", detector_id)
                                });

                            let chunker_id = detector_config.chunker_id.as_str();

                            let default_threshold = detector_config.default_threshold;

                            let chunk = chunks
                                .get(chunker_id)
                                .unwrap_or_else(|| panic!("chunk not found for {}", chunker_id))
                                .clone();

                            let headers = headers.clone();

                            async move {
                                detect_content(
                                    ctx,
                                    detector_id,
                                    default_threshold,
                                    detector_params,
                                    chunk,
                                    headers,
                                )
                                .await
                            }
                        })
                        .collect::<Vec<_>>(),
                )
                .await?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();

                detections.sort_by_key(|r| r.start);
                // Send result with detections
                Ok(TextContentDetectionResult { detections })
            }
            .instrument(Span::current()),
        );
        match task_handle.await {
            // Task completed successfully
            Ok(Ok(result)) => Ok(result),
            // Task failed, return error propagated from child task that failed
            Ok(Err(error)) => {
                error!(trace_id = ?task.trace_id, %error, "text content detection task failed");
                Err(error)
            }
            // Task cancelled or panicked
            Err(error) => {
                let error = error.into();
                error!(trace_id = ?task.trace_id, %error, "text content detection task failed");
                Err(error)
            }
        }
    }

    /// Handles context-related detections on textual content
    #[instrument(skip_all, fields(trace_id = ?task.trace_id, headers = ?task.headers))]
    pub async fn handle_context_documents_detection(
        &self,
        task: ContextDocsDetectionTask,
    ) -> Result<ContextDocsResult, Error> {
        info!(
            detectors = ?task.detectors,
            "handling context documents detection task"
        );
        let ctx = self.ctx.clone();
        let headers = task.headers;
        let task_handle = tokio::spawn(
            async move {
                // call detection
                let detections = try_join_all(
                    task.detectors
                        .iter()
                        .map(|(detector_id, detector_params)| {
                            let ctx = ctx.clone();
                            let detector_id = detector_id.clone();
                            let detector_params = detector_params.clone();
                            let content = task.content.clone();
                            let context_type = task.context_type.clone();
                            let context = task.context.clone();
                            let headers = headers.clone();

                            async {
                                detect_for_context(
                                    ctx,
                                    detector_id,
                                    detector_params,
                                    content,
                                    context_type,
                                    context,
                                    headers,
                                )
                                .await
                            }
                        })
                        .collect::<Vec<_>>(),
                )
                .await?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();

                Ok(ContextDocsResult { detections })
            }
            .instrument(Span::current()),
        );
        match task_handle.await {
            // Task completed successfully
            Ok(Ok(result)) => Ok(result),
            // Task failed, return error propagated from child task that failed
            Ok(Err(error)) => {
                error!(trace_id = ?task.trace_id, %error, "context documents detection task failed");
                Err(error)
            }
            // Task cancelled or panicked
            Err(error) => {
                let error = error.into();
                error!(trace_id = ?task.trace_id, %error, "context documents detection task failed");
                Err(error)
            }
        }
    }

    /// Handles detections on generated text (without performing generation)
    #[instrument(skip_all, fields(trace_id = ?task.trace_id, headers = ?task.headers))]
    pub async fn handle_generated_text_detection(
        &self,
        task: DetectionOnGenerationTask,
    ) -> Result<DetectionOnGenerationResult, Error> {
        info!(
            detectors = ?task.detectors,
            "handling detection on generated content task"
        );
        let ctx = self.ctx.clone();
        let headers = task.headers;

        let task_handle = tokio::spawn(
            async move {
                // call detection
                let detections = try_join_all(
                    task.detectors
                        .iter()
                        .map(|(detector_id, detector_params)| {
                            let ctx = ctx.clone();
                            let detector_id = detector_id.clone();
                            let detector_params = detector_params.clone();
                            let prompt = task.prompt.clone();
                            let generated_text = task.generated_text.clone();
                            let headers = headers.clone();
                            async {
                                detect_for_generation(
                                    ctx,
                                    detector_id,
                                    detector_params,
                                    prompt,
                                    generated_text,
                                    headers,
                                )
                                .await
                            }
                        })
                        .collect::<Vec<_>>(),
                )
                .await?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();

                Ok(DetectionOnGenerationResult { detections })
            }
            .instrument(Span::current()),
        );
        match task_handle.await {
            // Task completed successfully
            Ok(Ok(result)) => Ok(result),
            // Task failed, return error propagated from child task that failed
            Ok(Err(error)) => {
                error!(trace_id = ?task.trace_id, %error, "detection on generated content task failed");
                Err(error)
            }
            // Task cancelled or panicked
            Err(error) => {
                let error = error.into();
                error!(trace_id = ?task.trace_id, %error, "detection on generated content task failed");
                Err(error)
            }
        }
    }

    /// Handles detections on chat messages (without performing generation)
    #[instrument(skip_all, fields(trace_id = ?task.trace_id, headers = ?task.headers))]
    pub async fn handle_chat_detection(
        &self,
        task: ChatDetectionTask,
    ) -> Result<ChatDetectionResult, Error> {
        info!(
            detectors = ?task.detectors,
            "handling detection on chat content task"
        );
        let ctx = self.ctx.clone();
        let headers = task.headers;

        let task_handle = tokio::spawn(
            async move {
                // call detection
                let detections = try_join_all(
                    task.detectors
                        .iter()
                        .map(|(detector_id, detector_params)| {
                            let ctx = ctx.clone();
                            let detector_id = detector_id.clone();
                            let detector_params = detector_params.clone();
                            let messages = task.messages.clone();
                            let headers = headers.clone();
                            async {
                                detect_for_chat(
                                    ctx,
                                    detector_id,
                                    detector_params,
                                    messages,
                                    headers,
                                )
                                .await
                            }
                        })
                        .collect::<Vec<_>>(),
                )
                .await?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();

                Ok(ChatDetectionResult { detections })
            }
            .instrument(Span::current()),
        );
        match task_handle.await {
            // Task completed successfully
            Ok(Ok(result)) => Ok(result),
            // Task failed, return error propagated from child task that failed
            Ok(Err(error)) => {
                error!(%error, "detection task on chat failed");
                Err(error)
            }
            // Task cancelled or panicked
            Err(error) => {
                let error = error.into();
                error!(%error, "detection task on chat failed");
                Err(error)
            }
        }
    }
}

/// Handles input detection task.
#[instrument(skip_all)]
pub async fn input_detection_task(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    input_text: String,
    masks: Option<&[(usize, usize)]>,
    headers: HeaderMap,
) -> Result<Option<Vec<TokenClassificationResult>>, Error> {
    debug!(?detectors, "starting input detection");
    let text_with_offsets = apply_masks(input_text, masks);
    let chunker_ids = get_chunker_ids(ctx, detectors)?;
    let chunks = chunk_task(ctx, chunker_ids, text_with_offsets).await?;
    let detections = detection_task(ctx, detectors, chunks, headers).await?;
    Ok((!detections.is_empty()).then_some(detections))
}

/// Handles output detection task.
#[instrument(skip_all)]
async fn output_detection_task(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    generated_text: String,
    headers: HeaderMap,
) -> Result<Option<Vec<TokenClassificationResult>>, Error> {
    debug!(detectors = ?detectors.keys(), "starting output detection");
    let text_with_offsets = apply_masks(generated_text, None);
    let chunker_ids = get_chunker_ids(ctx, detectors)?;
    let chunks = chunk_task(ctx, chunker_ids, text_with_offsets).await?;
    let detections = detection_task(ctx, detectors, chunks, headers).await?;
    Ok((!detections.is_empty()).then_some(detections))
}

/// Handles detection task.
#[instrument(skip_all)]
async fn detection_task(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    chunks: HashMap<String, Vec<Chunk>>,
    headers: HeaderMap,
) -> Result<Vec<TokenClassificationResult>, Error> {
    debug!(detectors = ?detectors.keys(), "handling detection tasks");
    // Spawn tasks for each detector
    let tasks = detectors
        .iter()
        .map(|(detector_id, detector_params)| {
            let ctx = ctx.clone();
            let detector_id = detector_id.clone();
            let detector_params = detector_params.clone();
            // Get the detector config
            let detector_config = ctx
                .config
                .detectors
                .get(&detector_id)
                .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?;
            // Get the default threshold to use if threshold is not provided by the user
            let default_threshold = detector_config.default_threshold;
            // Get chunker for detector
            let chunker_id = detector_config.chunker_id.as_str();
            let chunks = chunks.get(chunker_id).unwrap().clone();
            let headers = headers.clone();
            Ok(tokio::spawn(
                async move {
                    detect(
                        ctx,
                        detector_id,
                        default_threshold,
                        detector_params,
                        chunks,
                        headers,
                    )
                    .await
                }
                .instrument(Span::current()),
            ))
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
#[instrument(skip_all)]
pub async fn chunk_task(
    ctx: &Arc<Context>,
    chunker_ids: Vec<String>,
    text_with_offsets: Vec<(usize, String)>,
) -> Result<HashMap<String, Vec<Chunk>>, Error> {
    debug!(?chunker_ids, "handling chunk task");
    // Spawn tasks for each chunker
    let tasks = chunker_ids
        .into_iter()
        .map(|chunker_id| {
            let ctx = ctx.clone();
            let text_with_offsets = text_with_offsets.clone();
            tokio::spawn(
                async move { chunk_parallel(&ctx, chunker_id, text_with_offsets).await }
                    .instrument(Span::current()),
            )
        })
        .collect::<Vec<_>>();
    let results = try_join_all(tasks)
        .await?
        .into_iter()
        .collect::<Result<HashMap<_, _>, Error>>()?;
    Ok(results)
}

/// Sends a request to a detector service and applies threshold.
#[instrument(skip_all, fields(detector_id))]
pub async fn detect(
    ctx: Arc<Context>,
    detector_id: String,
    default_threshold: f64,
    mut detector_params: DetectorParams,
    chunks: Vec<Chunk>,
    headers: HeaderMap,
) -> Result<Vec<TokenClassificationResult>, Error> {
    let detector_id = detector_id.clone();
    let threshold = detector_params.pop_threshold().unwrap_or(default_threshold);
    let contents: Vec<_> = chunks.iter().map(|chunk| chunk.text.clone()).collect();
    let response = if contents.is_empty() {
        // skip detector call as contents is empty
        Vec::default()
    } else {
        let request = ContentAnalysisRequest::new(contents, detector_params);
        debug!(?request, "sending detector request");
        let client = ctx
            .clients
            .get_as::<TextContentsDetectorClient>(&detector_id)
            .unwrap();
        client
            .text_contents(&detector_id, request, headers)
            .await
            .map_err(|error| {
                debug!(?error, "error received from detector");
                Error::DetectorRequestFailed {
                    id: detector_id.clone(),
                    error,
                }
            })?
    };
    debug!(?response, "received detector response");
    if chunks.len() != response.len() {
        return Err(Error::Other(format!(
            "Detector {detector_id} did not return expected number of responses"
        )));
    }
    let results = chunks
        .into_iter()
        .zip(response)
        .flat_map(|(chunk, response)| {
            response
                .into_iter()
                .filter_map(|resp| {
                    let mut result: TokenClassificationResult = resp.into();
                    // add detector_id
                    result.detector_id = Some(detector_id.clone());
                    result.start += chunk.offset as u32;
                    result.end += chunk.offset as u32;
                    (result.score >= threshold).then_some(result)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Ok::<Vec<TokenClassificationResult>, Error>(results)
}

/// Sends a request to a detector service and applies threshold.
/// TODO: Cleanup by removing duplicate code and merging it with above `detect` function
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_content(
    ctx: Arc<Context>,
    detector_id: String,
    default_threshold: f64,
    mut detector_params: DetectorParams,
    chunks: Vec<Chunk>,
    headers: HeaderMap,
) -> Result<Vec<ContentAnalysisResponse>, Error> {
    let detector_id = detector_id.clone();
    let threshold = detector_params.pop_threshold().unwrap_or(default_threshold);
    let contents: Vec<_> = chunks.iter().map(|chunk| chunk.text.clone()).collect();
    let response = if contents.is_empty() {
        // skip detector call as contents is empty
        Vec::default()
    } else {
        let request = ContentAnalysisRequest::new(contents, detector_params);
        debug!(?request, threshold, "sending detector request");
        let client = ctx
            .clients
            .get_as::<TextContentsDetectorClient>(&detector_id)
            .unwrap();
        client
            .text_contents(&detector_id, request, headers)
            .await
            .map_err(|error| {
                debug!(?error, "error received from detector");
                Error::DetectorRequestFailed {
                    id: detector_id.clone(),
                    error,
                }
            })?
    };
    debug!(%detector_id, ?response, "received detector response");
    if chunks.len() != response.len() {
        return Err(Error::Other(format!(
            "Detector {detector_id} did not return expected number of responses"
        )));
    }
    let results = chunks
        .into_iter()
        .zip(response)
        .flat_map(|(chunk, response)| {
            response
                .into_iter()
                .filter_map(|mut resp| {
                    resp.start += chunk.offset;
                    resp.end += chunk.offset;
                    // add detector_id
                    resp.detector_id = Some(detector_id.clone());
                    (resp.score >= threshold).then_some(resp)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Ok::<Vec<ContentAnalysisResponse>, Error>(results)
}

/// Calls a detector that implements the /api/v1/text/generation endpoint
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_for_generation(
    ctx: Arc<Context>,
    detector_id: String,
    mut detector_params: DetectorParams,
    prompt: String,
    generated_text: String,
    headers: HeaderMap,
) -> Result<Vec<DetectionResult>, Error> {
    let detector_id = detector_id.clone();
    let threshold = detector_params.pop_threshold().unwrap_or(
        detector_params.pop_threshold().unwrap_or(
            ctx.config
                .detectors
                .get(&detector_id)
                .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?
                .default_threshold,
        ),
    );
    let request =
        GenerationDetectionRequest::new(prompt.clone(), generated_text.clone(), detector_params);
    debug!(threshold, ?request, "sending generation detector request");
    let client = ctx
        .clients
        .get_as::<TextGenerationDetectorClient>(&detector_id)
        .unwrap_or_else(|| {
            panic!(
                "text generation detector client not found for {}",
                detector_id
            )
        });
    let response = client
        .text_generation(&detector_id, request, headers)
        .await
        .map(|results| {
            results
                .into_iter()
                .filter(|detection| detection.score > threshold)
                .map(|mut detection| {
                    // add detector_id
                    detection.detector_id = Some(detector_id.clone());
                    detection
                })
                .collect()
        })
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(?response, "received generation detector response");
    Ok::<Vec<DetectionResult>, Error>(response)
}

/// Calls a detector that implements the /api/v1/text/chat endpoint
pub async fn detect_for_chat(
    ctx: Arc<Context>,
    detector_id: String,
    mut detector_params: DetectorParams,
    messages: Vec<Message>,
    headers: HeaderMap,
) -> Result<Vec<DetectionResult>, Error> {
    let detector_id = detector_id.clone();
    let threshold = detector_params.pop_threshold().unwrap_or(
        detector_params.pop_threshold().unwrap_or(
            ctx.config
                .detectors
                .get(&detector_id)
                .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?
                .default_threshold,
        ),
    );
    let request = ChatDetectionRequest::new(messages.clone(), detector_params);
    debug!(%detector_id, ?request, "sending chat detector request");
    let client = ctx
        .clients
        .get_as::<TextChatDetectorClient>(&detector_id)
        .unwrap();
    let response = client
        .text_chat(&detector_id, request, headers)
        .await
        .map(|results| {
            results
                .into_iter()
                .filter(|detection| detection.score > threshold)
                .map(|mut detection| {
                    //add detector_id
                    detection.detector_id = Some(detector_id.clone());
                    detection
                })
                .collect()
        })
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(%detector_id, ?response, "received chat detector response");
    Ok::<Vec<DetectionResult>, Error>(response)
}

/// Calls a detector that implements the /api/v1/text/doc endpoint
#[instrument(skip_all, fields(detector_id))]
pub async fn detect_for_context(
    ctx: Arc<Context>,
    detector_id: String,
    mut detector_params: DetectorParams,
    content: String,
    context_type: ContextType,
    context: Vec<String>,
    headers: HeaderMap,
) -> Result<Vec<DetectionResult>, Error> {
    let detector_id = detector_id.clone();
    let threshold = detector_params.pop_threshold().unwrap_or(
        detector_params.pop_threshold().unwrap_or(
            ctx.config
                .detectors
                .get(&detector_id)
                .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?
                .default_threshold,
        ),
    );
    let request =
        ContextDocsDetectionRequest::new(content, context_type, context, detector_params.clone());
    debug!(
        ?request,
        threshold,
        ?detector_params,
        "sending context detector request"
    );
    let client = ctx
        .clients
        .get_as::<TextContextDocDetectorClient>(&detector_id)
        .unwrap_or_else(|| {
            panic!(
                "text context doc detector client not found for {}",
                detector_id
            )
        });
    let response = client
        .text_context_doc(&detector_id, request, headers)
        .await
        .map(|results| {
            results
                .into_iter()
                .filter(|detection| detection.score > threshold)
                .map(|mut detection| {
                    //add detector_id
                    detection.detector_id = Some(detector_id.clone());
                    detection
                })
                .collect()
        })
        .map_err(|error| Error::DetectorRequestFailed {
            id: detector_id.clone(),
            error,
        })?;
    debug!(%detector_id, ?response, "received context detector response");
    Ok::<Vec<DetectionResult>, Error>(response)
}

/// Sends request to chunker service.
#[instrument(skip_all, fields(chunker_id))]
pub async fn chunk(
    ctx: &Arc<Context>,
    chunker_id: String,
    offset: usize,
    text: String,
) -> Result<Vec<Chunk>, Error> {
    let request = chunkers::ChunkerTokenizationTaskRequest { text };
    debug!(?request, offset, "sending chunk request");
    let response = if chunker_id == DEFAULT_CHUNKER_ID {
        tokenize_whole_doc(request)
    } else {
        let client = ctx.clients.get_as::<ChunkerClient>(&chunker_id).unwrap();
        client
            .tokenization_task_predict(&chunker_id, request)
            .await
            .map_err(|error| Error::ChunkerRequestFailed {
                id: chunker_id.clone(),
                error,
            })?
    };

    debug!(?response, "received chunker response");
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
#[instrument(skip_all, fields(chunker_id))]
pub async fn chunk_parallel(
    ctx: &Arc<Context>,
    chunker_id: String,
    text_with_offsets: Vec<(usize, String)>,
) -> Result<(String, Vec<Chunk>), Error> {
    debug!("sending parallel chunk requests");
    let chunks = stream::iter(text_with_offsets)
        .map(|(offset, text)| {
            let ctx = ctx.clone();
            let chunker_id = chunker_id.clone();
            async move {
                let results = chunk(&ctx, chunker_id, offset, text).await?;
                Ok::<Vec<Chunk>, Error>(results)
            }
        })
        .buffered(DEFAULT_STREAM_BUFFER_SIZE)
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
#[instrument(skip_all, fields(model_id))]
pub async fn tokenize(
    ctx: &Arc<Context>,
    model_id: String,
    text: String,
    headers: HeaderMap,
) -> Result<(u32, Vec<String>), Error> {
    debug!("sending tokenize request");
    let client = ctx
        .clients
        .get_as::<GenerationClient>("generation")
        .unwrap();
    client
        .tokenize(model_id.clone(), text, headers)
        .await
        .map_err(|error| Error::TokenizeRequestFailed {
            id: model_id,
            error,
        })
}

/// Sends generate request to a generation service.
#[instrument(skip_all, fields(model_id))]
async fn generate(
    ctx: &Arc<Context>,
    model_id: String,
    text: String,
    params: Option<GuardrailsTextGenerationParameters>,
    headers: HeaderMap,
) -> Result<ClassifiedGeneratedTextResult, Error> {
    debug!("sending generate request");
    let client = ctx
        .clients
        .get_as::<GenerationClient>("generation")
        .unwrap();
    client
        .generate(model_id.clone(), text, params, headers)
        .await
        .map_err(|error| Error::GenerateRequestFailed {
            id: model_id,
            error,
        })
}

#[cfg(test)]
mod tests {
    use hyper::{HeaderMap, StatusCode};

    use super::*;
    use crate::{
        clients::{
            self,
            detector::{ContentAnalysisResponse, GenerationDetectionRequest},
            ClientMap, GenerationClient, TgisClient,
        },
        config::{DetectorConfig, OrchestratorConfig},
        models::{DetectionResult, EvidenceObj, FinishReason, THRESHOLD_PARAM},
        pb::fmaas::{
            BatchedGenerationRequest, BatchedGenerationResponse, GenerationRequest,
            GenerationResponse, StopReason,
        },
    };

    // Test for TGIS generation with default parameter
    #[tokio::test]
    async fn test_tgis_generate_with_default_params() {
        let mut tgis_client = TgisClient::faux();

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
        faux::when!(tgis_client.generate(expected_generate_req_args, HeaderMap::new()))
            .once() // TODO: Add with_args
            .then_return(Ok(client_generation_response));

        let generation_client = GenerationClient::tgis(tgis_client.clone());

        let mut clients = ClientMap::new();
        clients.insert("generation".into(), generation_client);
        let ctx = Arc::new(Context::new(OrchestratorConfig::default(), clients));

        // Test request formulation and response processing is as expected
        assert_eq!(
            generate(&ctx, text_gen_model_id, sample_text, None, HeaderMap::new())
                .await
                .unwrap(),
            expected_generate_response
        );
    }

    /// This test checks if calls to detectors are being handled appropriately.
    /// It receives an input of two chunks. The first sentence does not contain a
    /// detection. The second one does.
    ///
    /// The idea behind this test case is to test that...
    /// 1. offsets are calculated correctly.
    /// 2. detections below the threshold are not returned to the client.
    #[tokio::test]
    async fn test_handle_detection_task() {
        let generation_client = GenerationClient::tgis(TgisClient::faux());
        let mut detector_client = TextContentsDetectorClient::faux();

        let detector_id = "mocked_hap_detector";
        let threshold = 0.5;
        // Input: "I don't like potatoes. I hate aliens.";
        let first_sentence = "I don't like potatoes.".to_string();
        let second_sentence = "I hate aliens.".to_string();
        let mut detector_params = DetectorParams::new();
        detector_params.insert(THRESHOLD_PARAM.into(), threshold.into());
        let chunks = vec![
            Chunk {
                offset: 0,
                text: first_sentence.clone(),
            },
            Chunk {
                offset: 23,
                text: second_sentence.clone(),
            },
        ];

        // Since only the second chunk has a detection, we only expect one detection in the output.
        let expected_response: Vec<TokenClassificationResult> = vec![TokenClassificationResult {
            start: 23,
            end: 37,
            word: second_sentence.clone(),
            entity: "has_HAP".to_string(),
            entity_group: "hap".to_string(),
            detector_id: Some(detector_id.to_string()),
            score: 0.9,
            token_count: None,
        }];

        faux::when!(detector_client.text_contents(
            detector_id,
            ContentAnalysisRequest::new(
                vec![first_sentence.clone(), second_sentence.clone()],
                DetectorParams::new()
            ),
            HeaderMap::new(),
        ))
        .once()
        .then_return(Ok(vec![
            vec![ContentAnalysisResponse {
                start: 0,
                end: 22,
                text: first_sentence.clone(),
                detection: "has_HAP".to_string(),
                detection_type: "hap".to_string(),
                detector_id: Some(detector_id.to_string()),
                score: 0.1,
                evidence: Some(vec![]),
            }],
            vec![ContentAnalysisResponse {
                start: 0,
                end: 14,
                text: second_sentence.clone(),
                detection: "has_HAP".to_string(),
                detection_type: "hap".to_string(),
                detector_id: Some(detector_id.to_string()),
                score: 0.9,
                evidence: Some(vec![]),
            }],
        ]));

        let mut clients = ClientMap::new();
        clients.insert("generation".into(), generation_client);
        clients.insert(detector_id.into(), detector_client);
        let ctx = Arc::new(Context::new(OrchestratorConfig::default(), clients));

        assert_eq!(
            detect(
                ctx,
                detector_id.to_string(),
                threshold,
                detector_params,
                chunks,
                HeaderMap::new(),
            )
            .await
            .unwrap(),
            expected_response
        );
    }

    /// This test checks if calls to detectors returning 503 are being propagated in the orchestrator response.
    #[tokio::test]
    async fn test_detect_when_detector_returns_503() {
        let generation_client = GenerationClient::tgis(TgisClient::faux());
        let mut detector_client = TextContentsDetectorClient::faux();

        let detector_id = "mocked_503_detector";
        let sentence = "This call will return a 503.".to_string();
        let threshold = 0.5;
        let mut detector_params = DetectorParams::new();
        detector_params.insert(THRESHOLD_PARAM.into(), threshold.into());
        let chunks = vec![Chunk {
            offset: 0,
            text: sentence.clone(),
        }];

        // We expect the detector call to return a 503, with a response complying with the error response.
        let expected_response = Error::DetectorRequestFailed {
            id: detector_id.to_string(),
            error: clients::Error::Http {
                code: StatusCode::SERVICE_UNAVAILABLE,
                message: "Service Unavailable".to_string(),
            },
        };

        faux::when!(detector_client.text_contents(
            detector_id,
            ContentAnalysisRequest::new(vec![sentence.clone()], DetectorParams::new()),
            HeaderMap::new(),
        ))
        .once()
        .then_return(Err(clients::Error::Http {
            code: StatusCode::SERVICE_UNAVAILABLE,
            message: "Service Unavailable".to_string(),
        }));

        let mut clients = ClientMap::new();
        clients.insert("generation".into(), generation_client);
        clients.insert(detector_id.into(), detector_client);
        let ctx = Arc::new(Context::new(OrchestratorConfig::default(), clients));

        assert_eq!(
            detect(
                ctx,
                detector_id.to_string(),
                threshold,
                detector_params,
                chunks,
                HeaderMap::new(),
            )
            .await
            .unwrap_err(),
            expected_response
        );
    }

    #[tokio::test]
    async fn test_handle_detection_task_with_whitespace() {
        let generation_client = GenerationClient::tgis(TgisClient::faux());
        let mut detector_client = TextContentsDetectorClient::faux();

        let detector_id = "mocked_hap_detector";
        let threshold = 0.5;
        let first_sentence = "".to_string();
        let mut detector_params = DetectorParams::new();
        detector_params.insert(THRESHOLD_PARAM.into(), threshold.into());
        let chunks = vec![Chunk {
            offset: 0,
            text: first_sentence.clone(),
        }];

        faux::when!(detector_client.text_contents(
            detector_id,
            ContentAnalysisRequest::new(vec![first_sentence.clone()], DetectorParams::new()),
            HeaderMap::new(),
        ))
        .once()
        .then_return(Ok(vec![vec![]]));

        let mut clients = ClientMap::new();
        clients.insert("generation".into(), generation_client);
        clients.insert(detector_id.into(), detector_client);
        let ctx = Arc::new(Context::new(OrchestratorConfig::default(), clients));

        let expected_response_whitespace = vec![];
        assert_eq!(
            detect(
                ctx,
                detector_id.to_string(),
                threshold,
                detector_params,
                chunks,
                HeaderMap::new(),
            )
            .await
            .unwrap(),
            expected_response_whitespace
        );
    }

    #[tokio::test]
    async fn test_detect_for_generation() {
        let generation_client = GenerationClient::tgis(TgisClient::faux());
        let mut detector_client = TextGenerationDetectorClient::faux();

        let detector_id = "mocked_answer_relevance_detector";
        let threshold = 0.5;
        let prompt = "What is the capital of Brazil?".to_string();
        let generated_text = "The capital of Brazil is Brasilia.".to_string();
        let mut detector_params = DetectorParams::new();
        detector_params.insert(THRESHOLD_PARAM.into(), threshold.into());

        let expected_response: Vec<DetectionResult> = vec![DetectionResult {
            detection_type: "relevance".to_string(),
            detection: "is_relevant".to_string(),
            detector_id: Some(detector_id.to_string()),
            score: 0.9,
            evidence: Some(
                [EvidenceObj {
                    name: "relevant chunk".into(),
                    value: Some("What is capital of Brazil".into()),
                    score: Some(0.99),
                    evidence: None,
                }]
                .to_vec(),
            ),
        }];

        faux::when!(detector_client.text_generation(
            detector_id,
            GenerationDetectionRequest::new(
                prompt.clone(),
                generated_text.clone(),
                DetectorParams::new()
            ),
            HeaderMap::new(),
        ))
        .once()
        .then_return(Ok(vec![DetectionResult {
            detection_type: "relevance".to_string(),
            detection: "is_relevant".to_string(),
            detector_id: Some(detector_id.to_string()),
            score: 0.9,
            evidence: Some(
                [EvidenceObj {
                    name: "relevant chunk".into(),
                    value: Some("What is capital of Brazil".into()),
                    score: Some(0.99),
                    evidence: None,
                }]
                .to_vec(),
            ),
        }]));

        let mut clients = ClientMap::new();
        clients.insert("generation".into(), generation_client);
        clients.insert(detector_id.into(), detector_client);
        let mut ctx = Context::new(OrchestratorConfig::default(), clients);
        // add detector
        ctx.config.detectors.insert(
            detector_id.to_string(),
            DetectorConfig {
                ..Default::default()
            },
        );
        let ctx = Arc::new(ctx);

        assert_eq!(
            detect_for_generation(
                ctx,
                detector_id.to_string(),
                detector_params,
                prompt,
                generated_text,
                HeaderMap::new(),
            )
            .await
            .unwrap(),
            expected_response
        );
    }

    #[tokio::test]
    async fn test_detect_for_generation_below_threshold() {
        let generation_client = GenerationClient::tgis(TgisClient::faux());
        let mut detector_client = TextGenerationDetectorClient::faux();

        let detector_id = "mocked_answer_relevance_detector";
        let threshold = 0.5;
        let prompt = "What is the capital of Brazil?".to_string();
        let generated_text =
            "The most beautiful places can be found in Rio de Janeiro.".to_string();
        let mut detector_params = DetectorParams::new();
        detector_params.insert(THRESHOLD_PARAM.into(), threshold.into());

        let expected_response: Vec<DetectionResult> = vec![];

        faux::when!(detector_client.text_generation(
            detector_id,
            GenerationDetectionRequest::new(
                prompt.clone(),
                generated_text.clone(),
                DetectorParams::new()
            ),
            HeaderMap::new(),
        ))
        .once()
        .then_return(Ok(vec![DetectionResult {
            detection_type: "relevance".to_string(),
            detection: "is_relevant".to_string(),
            detector_id: Some(detector_id.to_string()),
            score: 0.1,
            evidence: None,
        }]));

        let mut clients = ClientMap::new();
        clients.insert("generation".into(), generation_client);
        clients.insert(detector_id.into(), detector_client);
        let mut ctx = Context::new(OrchestratorConfig::default(), clients);
        // add detector
        ctx.config.detectors.insert(
            detector_id.to_string(),
            DetectorConfig {
                ..Default::default()
            },
        );
        let ctx = Arc::new(ctx);

        assert_eq!(
            detect_for_generation(
                ctx,
                detector_id.to_string(),
                detector_params,
                prompt,
                generated_text,
                HeaderMap::new(),
            )
            .await
            .unwrap(),
            expected_response
        );
    }
}
