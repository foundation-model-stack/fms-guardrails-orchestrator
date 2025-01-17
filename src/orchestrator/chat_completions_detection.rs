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
use axum::http::HeaderMap;
use futures::future::{join_all, try_join_all};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, sync::Arc};
use tracing::{debug, info, instrument};

use super::{ChatCompletionsDetectionTask, Context, Error, Orchestrator};
use crate::clients::openai::OrchestratorWarning;
use crate::{
    clients::{
        detector::{ChatDetectionRequest, ContentAnalysisRequest},
        openai::{
            ChatCompletion, ChatCompletionChoice, ChatCompletionsRequest, ChatCompletionsResponse,
            ChatDetections, Content, InputDetectionResult, OpenAiClient,
        },
    },
    config::DetectorType,
    models::{DetectorParams, GuardrailDetection, InputWarningReason},
    orchestrator::{
        detector_processing::content,
        unary::{chunk, detect_content},
        Chunk, UNSUITABLE_INPUT_MESSAGE,
    },
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Internal structure to capture chat messages (both request and response)
/// and prepare it for processing
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessageInternal {
    /// Index of the message
    pub message_index: usize,
    /// The role of the messages author.
    pub role: String,
    /// The contents of the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    /// The refusal message by the assistant. (assistant message only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

pub enum DetectorRequest {
    ContentAnalysisRequest(ContentAnalysisRequest),
    ChatDetectionRequest(ChatDetectionRequest),
}

// Get Vec<ChatMessageInternal> from ChatCompletionsRequest
impl From<ChatCompletionsRequest> for Vec<ChatMessageInternal> {
    fn from(value: ChatCompletionsRequest) -> Self {
        value
            .messages
            .into_iter()
            .enumerate()
            .map(|(index, message)| ChatMessageInternal {
                message_index: index,
                role: message.role,
                content: message.content,
                refusal: message.refusal,
            })
            .collect()
    }
}

// Get Vec<ChatMessageInternal> from ChatCompletionChoice
impl From<ChatCompletionChoice> for Vec<ChatMessageInternal> {
    fn from(value: ChatCompletionChoice) -> Self {
        vec![ChatMessageInternal {
            message_index: value.index,
            role: value.message.role,
            content: Some(Content::Text(value.message.content.unwrap_or_default())),
            refusal: value.message.refusal,
        }]
    }
}

// TODO: Add from function for streaming response as well

impl Orchestrator {
    #[instrument(skip_all, fields(trace_id = ?task.trace_id, headers = ?task.headers))]

    pub async fn handle_chat_completions_detection(
        &self,
        task: ChatCompletionsDetectionTask,
    ) -> Result<ChatCompletionsResponse, Error> {
        info!("handling chat completions detection task");
        let ctx = self.ctx.clone();
        let headers = task.headers.clone();

        let request = task.request.clone();
        let task_handle = tokio::spawn(async move {
            // Convert the request into a format that can be used for processing
            let chat_messages = Vec::<ChatMessageInternal>::from(request.clone());
            let input_detectors = request.detectors.unwrap_or_default().input;

            let input_detections = match input_detectors {
                Some(detectors) if !detectors.is_empty() => {
                    // Call out to input detectors using chunk
                    input_detection(&ctx, &detectors, chat_messages, headers.clone()).await?
                }
                _ => None,
            };

            debug!(?input_detections);

            if let Some(mut input_detections) = input_detections {
                // Sort input detections by message_index
                input_detections.sort_by_key(|value| value.message_index);

                let input_detections = input_detections
                    .into_iter()
                    .map(|mut detection| {
                        let last_idx = detection.results.len();
                        // sort detection by starting span, if span is not present then move to the end of the message
                        detection.results.sort_by_key(|r| match r {
                            GuardrailDetection::ContentAnalysisResponse(value) => value.start,
                            _ => last_idx,
                        });
                        detection
                    })
                    .collect::<Vec<_>>();

                Ok(ChatCompletionsResponse::Unary(Box::new(ChatCompletion {
                    id: Uuid::new_v4().simple().to_string(),
                    model: request.model,
                    choices: vec![],
                    created: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64,
                    detections: Some(ChatDetections {
                        input: input_detections,
                        output: vec![],
                    }),
                    warnings: vec![OrchestratorWarning::new(
                        InputWarningReason::UnsuitableInput,
                        UNSUITABLE_INPUT_MESSAGE,
                    )],
                    ..Default::default()
                })))
            } else {
                let model_id = task.request.model.clone();
                let client = ctx
                    .clients
                    .get_as::<OpenAiClient>("chat_generation")
                    .expect("chat_generation client not found");
                let mut chat_request = task.request;
                // Remove detectors as chat completion server would reject extra parameter
                chat_request.detectors = None;
                client
                    .chat_completions(chat_request, task.headers)
                    .await
                    .map_err(|error| Error::ChatGenerateRequestFailed {
                        id: model_id,
                        error,
                    })
            }
        });

        match task_handle.await {
            // Task completed successfully
            Ok(Ok(result)) => Ok(result),
            // Task failed, return error propagated from child task that failed
            Ok(Err(error)) => {
                // TODO: Transform error from chat completion client
                Err(error)
            }
            // Task cancelled or panicked
            Err(error) => {
                // TODO: Transform error from chat completion client
                let error = error.into();
                Err(error)
            }
        }
    }
}

#[instrument(skip_all)]
pub async fn input_detection(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    chat_messages: Vec<ChatMessageInternal>,
    headers: HeaderMap,
) -> Result<Option<Vec<InputDetectionResult>>, Error> {
    debug!(?detectors, "starting input detection on chat completions");

    let ctx = ctx.clone();

    // pre-process chat messages based on individual detectors to prepare for chunking
    let processed_chat_messages = preprocess_chat_messages(&ctx, detectors, chat_messages)?;

    // Call out to the chunker to get chunks of messages based on detector type
    let chunks = detector_chunk_task(&ctx, processed_chat_messages).await?;

    // We run over each detector and take out messages that are appropriate for that detector.
    let tasks = detectors
        .iter()
        .flat_map(|(detector_id, detector_params)| {
            let detector_id = detector_id.clone();
            let detector_config = ctx
                .config
                .detectors
                .get(&detector_id)
                .unwrap_or_else(|| panic!("detector config not found for {}", detector_id));
            let default_threshold = detector_config.default_threshold;

            let detector_type = &detector_config.r#type;

            // Get chunks corresponding to each message
            let messages = chunks.get(&detector_id).unwrap().clone();

            match detector_type {
                DetectorType::TextContents => {
                    // spawn parallel processes for each message index and run detection on them.
                    messages
                        .into_iter()
                        .map(|(idx, chunks)| {
                            let ctx = ctx.clone();
                            let detector_id = detector_id.clone();
                            let detector_params = detector_params.clone();
                            let headers = headers.clone();

                            tokio::spawn(async move {
                                // Call content detector on the chunks of particular message
                                // and return the index and detection results
                                let result = detect_content(
                                    ctx.clone(),
                                    detector_id.clone(),
                                    default_threshold,
                                    detector_params.clone(),
                                    chunks,
                                    headers.clone(),
                                )
                                .await;
                                match result {
                                    Ok(value) => {
                                        if !value.is_empty() {
                                            let input_detection_result = InputDetectionResult {
                                                message_index: idx,
                                                results: value
                                                    .into_iter()
                                                    .map(|result| {
                                                        GuardrailDetection::ContentAnalysisResponse(
                                                            result,
                                                        )
                                                    })
                                                    .collect::<Vec<_>>(),
                                            };
                                            Ok(input_detection_result)
                                        } else {
                                            Ok(InputDetectionResult {
                                                message_index: idx,
                                                results: vec![],
                                            })
                                        }
                                    }
                                    Err(error) => Err(error),
                                }
                            })
                        })
                        .collect::<Vec<_>>()
                }
                _ => unimplemented!(),
            }
        })
        .collect::<Vec<_>>();

    let detections = try_join_all(tasks)
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, Error>>()?
        .into_iter()
        .filter(|detection| !detection.results.is_empty())
        .collect::<Vec<InputDetectionResult>>();

    Ok((!detections.is_empty()).then_some(detections))
}

/// Function to filter messages based on individual detectors
/// Returns a HashMap of detector id to filtered messages
fn preprocess_chat_messages(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    messages: Vec<ChatMessageInternal>,
) -> Result<HashMap<String, Vec<ChatMessageInternal>>, Error> {
    detectors
        .iter()
        .map(
            |(detector_id, _)| -> Result<(String, Vec<ChatMessageInternal>), Error> {
                let ctx = ctx.clone();
                let detector_id = detector_id.clone();
                let detector_config = ctx
                    .config
                    .detectors
                    .get(&detector_id)
                    .unwrap_or_else(|| panic!("detector config not found for {}", detector_id));
                let detector_type = &detector_config.r#type;
                // Filter messages based on detector type
                let messages = match detector_type {
                    DetectorType::TextContents => content::filter_chat_message(&messages),
                    _ => unimplemented!(),
                }?;
                Ok((detector_id, messages))
            },
        )
        .collect()
}

// Function to chunk Vec<ChatMessageInternal> based on the chunker id and return chunks in Vec<ChatMessageInternal> form
// Output maps each detector_id with corresponding chunk
async fn detector_chunk_task(
    ctx: &Arc<Context>,
    detector_chat_messages: HashMap<String, Vec<ChatMessageInternal>>,
) -> Result<HashMap<String, Vec<(usize, Vec<Chunk>)>>, Error> {
    let mut chunks = HashMap::new();

    // TODO: Improve error handling for the code below
    for (detector_id, chat_messages) in detector_chat_messages.into_iter() {
        let chunk_tasks = chat_messages
            .into_iter()
            .map(|message| {
                let Some(Content::Text(text)) = message.content else {
                    panic!("Only text content accepted")
                };
                let offset: usize = 0;
                let task = tokio::spawn({
                    let detector_id = detector_id.clone();
                    let ctx = ctx.clone();
                    async move {
                        let chunker_id = ctx.config.get_chunker_id(&detector_id).unwrap();
                        chunk(&ctx, chunker_id, offset, text).await
                    }
                });
                // Return tuple of message index and task
                (message.message_index, task)
                // chunking_tasks.push((detector_id, task));
            })
            .collect::<Vec<_>>();

        let results = join_all(chunk_tasks.into_iter().map(|(index, handle)| async move {
            match handle.await {
                Ok(Ok(value)) => Ok((index, value)), // Success
                Ok(Err(err)) => {
                    // Task returned an error
                    Err(err)
                }
                Err(_) => {
                    // Chunking failed
                    Err(Error::Other("Chunking task failed".to_string()))
                }
            }
        }))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, Error>>();

        match results {
            Ok(chunk_value) => {
                chunks.insert(detector_id.clone(), chunk_value);
                Ok(())
            }
            Err(err) => Err(err),
        }?
    }

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use std::any::{Any, TypeId};

    use super::*;
    use crate::config::DetectorConfig;
    use crate::orchestrator::{ClientMap, OrchestratorConfig};

    // Test to verify preprocess_chat_messages works correctly for multiple content type detectors
    // with single message in chat request
    #[tokio::test]
    async fn pretest_process_chat_messages_multiple_content_detector() {
        // Test setup
        let clients = ClientMap::new();
        let detector_1_id = "detector1";
        let detector_2_id = "detector2";
        let mut ctx = Context::new(OrchestratorConfig::default(), clients);
        // add detector
        ctx.config.detectors.insert(
            detector_1_id.to_string().clone(),
            DetectorConfig {
                ..Default::default()
            },
        );
        ctx.config.detectors.insert(
            detector_2_id.to_string().clone(),
            DetectorConfig {
                ..Default::default()
            },
        );

        let ctx = Arc::new(ctx);
        let mut detectors = HashMap::new();
        detectors.insert(detector_1_id.to_string(), DetectorParams::new());
        detectors.insert(detector_2_id.to_string(), DetectorParams::new());

        let messages = vec![ChatMessageInternal {
            message_index: 0,
            content: Some(Content::Text("hello".to_string())),
            role: "assistant".to_string(),
            ..Default::default()
        }];
        let processed_messages = preprocess_chat_messages(&ctx, &detectors, messages).unwrap();
        // Assertions
        assert!(processed_messages[detector_1_id].len() == 1);
        assert!(processed_messages[detector_2_id].len() == 1);
    }

    // Test preprocess_chat_messages returns error correctly for multiple content type detectors
    // with incorrect message requirements
    #[tokio::test]
    async fn pretest_process_chat_messages_error_handling() {
        // Test setup
        let clients = ClientMap::new();
        let detector_1_id = "detector1";
        let mut ctx = Context::new(OrchestratorConfig::default(), clients);
        // add detector
        ctx.config.detectors.insert(
            detector_1_id.to_string().clone(),
            DetectorConfig {
                ..Default::default()
            },
        );

        let ctx = Arc::new(ctx);
        let mut detectors = HashMap::new();
        detectors.insert(detector_1_id.to_string(), DetectorParams::new());

        let messages = vec![ChatMessageInternal {
            message_index: 0,
            content: Some(Content::Text("hello".to_string())),
            // Invalid role will return error used for testing
            role: "foo".to_string(),
            ..Default::default()
        }];

        let processed_messages = preprocess_chat_messages(&ctx, &detectors, messages);

        // Assertions
        assert!(processed_messages.is_err());
        let error = processed_messages.unwrap_err();
        assert_eq!(error.type_id(), TypeId::of::<Error>());
        assert_eq!(
            error.to_string(),
            "validation error: Message at last index is not from user or assistant or system"
        );
    }
}
