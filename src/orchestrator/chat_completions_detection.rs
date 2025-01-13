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
use futures::{
    future::{join_all, try_join_all},
    Future,
};
use std::{collections::HashMap, sync::Arc};
use std::{
    pin::Pin,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, info, instrument};

use super::{ChatCompletionsDetectionTask, Context, Error, Orchestrator};
use crate::{
    clients::{
        detector::{ChatDetectionRequest, ContentAnalysisRequest},
        openai::{
            ChatCompletion, ChatCompletionChoice, ChatCompletionsRequest, ChatCompletionsResponse,
            Content, DetectionResult, InputDetectionResult, OpenAiClient,
        },
    },
    config::DetectorType,
    models::{DetectorParams, OrchestratorDetectionResult},
    orchestrator::{
        detector_processing::content,
        unary::{chunk, detect_content},
        Chunk,
    },
};
use serde::{Deserialize, Serialize};

pub type ChunkResult<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Internal structure to capture chat messages (both request and response)
/// and prepare it for processing
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessageInternal {
    // Index of the message
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

pub type ChatMessagesInternal = Vec<ChatMessageInternal>;
pub enum DetectorRequest {
    ContentAnalysisRequest(ContentAnalysisRequest),
    ChatDetectionRequest(ChatDetectionRequest),
}

// Get ChatMessagesInternal from ChatCompletionsRequest
impl From<ChatCompletionsRequest> for ChatMessagesInternal {
    fn from(value: ChatCompletionsRequest) -> Self {
        let mut messages = ChatMessagesInternal::new();
        value.messages.iter().enumerate().for_each(|(index, m)| {
            messages.push({
                ChatMessageInternal {
                    message_index: index,
                    role: m.role.clone(),
                    content: m.content.clone(),
                    refusal: m.refusal.clone(),
                }
            })
        });
        messages
    }
}

// Get ChatMessagesInternal from ChatCompletionChoice
impl From<ChatCompletionChoice> for ChatMessagesInternal {
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
            let chat_messages = ChatMessagesInternal::from(request.clone());
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
                    .map(|detection| {
                        let last_idx = detection.results.clone().unwrap().len();
                        // sort detection by starting span, if span is not present then move to the end of the message
                        detection.results.clone().unwrap().sort_by_key(|r| match r {
                            OrchestratorDetectionResult::ContentAnalysisResponse(value) => {
                                value.start
                            }
                            _ => last_idx,
                        });
                        detection
                    })
                    .collect::<Vec<_>>();

                Ok(ChatCompletionsResponse::Unary(Box::new(ChatCompletion {
                    id: "dummy-id".to_string(), // TODO: Replace with real unique ID generator
                    model: request.model,
                    choices: vec![],
                    created: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64,
                    detections: Some(DetectionResult {
                        input: Some(input_detections),
                        output: None,
                    }),
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
    chat_messages: ChatMessagesInternal,
    headers: HeaderMap,
) -> Result<Option<Vec<InputDetectionResult>>, Error> {
    debug!(?detectors, "starting input detection on chat completions");

    let ctx = ctx.clone();

    // filter chat messages based on individual detectors to prepare for chunking
    let filtered_chat_messages = filter_chat_messages(&ctx, detectors, chat_messages)?;

    // Call out to the chunker to get chunks of messages based on detector type
    let chunks = detector_chunk_task(&ctx, filtered_chat_messages).await?;

    // We run over each detector and take out messages that are appropriate for that detector.
    let tasks = detectors
        .iter()
        .flat_map(|(detector_id, detector_params)| {
            let detector_id = detector_id.clone();
            let detector_config =
                ctx.config.detectors.get(&detector_id).unwrap_or_else(|| {
                    panic!("detector config not found for {}", detector_id)
                });
            let default_threshold = detector_config.default_threshold;

            let detector_type = &detector_config.r#type;

            // Get chunks corresponding to each message
            let messages = chunks
                .get(&detector_id)
                .unwrap()
                .clone();

            match detector_type {
                DetectorType::TextContents => {

                    // spawn parallel processes for each message index and run detection on them.
                    let tasks = messages
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
                            ).await;
                            match result {
                                Ok(value) => {
                                    if !value.is_empty() {
                                        let input_detection_result = InputDetectionResult {
                                            message_index: idx as u16,
                                            results: Some(
                                                value
                                                .iter()
                                                .map(|result| {OrchestratorDetectionResult::ContentAnalysisResponse(result.clone())})
                                                .collect::<Vec<_>>())
                                        };
                                        Ok(input_detection_result)
                                    }
                                    else {
                                        Ok(InputDetectionResult { message_index: idx as u16, results: None })
                                    }
                                },
                                Err(error) => Err(error)
                            }
                        })
                    })
                    .collect::<Vec<_>>();
                    tasks

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
        .filter(|detection| detection.results.is_some())
        .collect::<Vec<InputDetectionResult>>();

    Ok((!detections.is_empty()).then_some(detections))
}

/// Function to filter messages based on individual detectors
/// Returns a HashMap of detector id to filtered messages
fn filter_chat_messages(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    messages: ChatMessagesInternal,
) -> Result<HashMap<String, ChatMessagesInternal>, Error> {
    detectors
        .iter()
        .map(
            |(detector_id, _)| -> Result<(String, ChatMessagesInternal), Error> {
                let ctx = ctx.clone();
                let detector_id = detector_id.clone();
                let detector_config = ctx
                    .config
                    .detectors
                    .get(&detector_id)
                    .unwrap_or_else(|| panic!("detector config not found for {}", detector_id));
                let detector_type = &detector_config.r#type;
                // Filter messages based on detector type
                let value = match detector_type {
                    DetectorType::TextContents => {
                        match content::filter_chat_message(messages.clone()) {
                            Ok(filtered_messages) => Ok(filtered_messages),
                            Err(e) => Err(Error::ValidationFailed {
                                id: detector_id.clone(),
                                error: e.to_string(),
                            }),
                        }
                    }
                    _ => unimplemented!(),
                }?;
                Ok((detector_id, value))
            },
        )
        .collect::<Result<HashMap<String, ChatMessagesInternal>, Error>>()
}

// Function to chunk ChatMessagesInternal based on the chunker id and return chunks in ChatMessagesInternal form
// Output maps each detector_id with corresponding chunk
async fn detector_chunk_task(
    ctx: &Arc<Context>,
    detector_chat_messages: HashMap<String, ChatMessagesInternal>,
) -> Result<HashMap<String, Vec<(usize, Vec<Chunk>)>>, Error> {
    let mut chunks = HashMap::<String, Vec<(usize, Vec<Chunk>)>>::new();

    // TODO: Improve error handling for the code below
    for (detector_id, chat_messages) in detector_chat_messages.iter() {
        let chunk_tasks = chat_messages
            .iter()
            .map(|message| {
                let text = match message.content.as_ref().unwrap() {
                    Content::Text(value) => value,
                    _ => panic!("Only text content accepted"),
                };
                let offset: usize = 0;
                let task = tokio::spawn({
                    let detector_id = detector_id.clone();
                    let text = text.clone();
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
                let chunk_values = chunk_value.into_iter().collect::<Vec<_>>();
                chunks.insert(detector_id.clone(), chunk_values);
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

    // Test to verify filter_chat_message works correctly for multiple content type detectors
    // with single message in chat request
    #[tokio::test]
    async fn test_filter_chat_messages_multiple_content_detector() {
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
        let filtered_messages = filter_chat_messages(&ctx, &detectors, messages).unwrap();
        // Assertions
        assert!(filtered_messages[detector_1_id].len() == 1);
        assert!(filtered_messages[detector_2_id].len() == 1);
    }

    // Test filter_chat_message returns error correctly for multiple content type detectors
    // with incorrect message requirements
    #[tokio::test]
    async fn test_filter_chat_messages_error_handling() {
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

        let filtered_messages = filter_chat_messages(&ctx, &detectors, messages);

        // Assertions
        assert!(filtered_messages.is_err());
        let error = filtered_messages.unwrap_err();
        assert_eq!(error.type_id(), TypeId::of::<Error>());
        assert_eq!(
            error.to_string(),
            "validation failed for `detector1`: Message at last index is not from user or assistant"
        );
    }
}
