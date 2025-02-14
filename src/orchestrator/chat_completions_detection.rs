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
use std::{
    collections::{btree_map, BTreeMap, HashMap},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::http::HeaderMap;
use futures::future::{join_all, try_join_all};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};
use uuid::Uuid;

use super::{
    ChatCompletionsDetectionTask, Context, Error, Orchestrator, UNSUITABLE_OUTPUT_MESSAGE,
};
use crate::{
    clients::{
        detector::{ChatDetectionRequest, ContentAnalysisRequest, ContentAnalysisResponse},
        openai::{
            ChatCompletion, ChatCompletionChoice, ChatCompletionsRequest, ChatCompletionsResponse,
            ChatDetections, Content, DetectionResult, InputDetectionResult, OpenAiClient,
            OrchestratorWarning, OutputDetectionResult, Role,
        },
    },
    config::DetectorType,
    models::{DetectionWarningReason, DetectorParams},
    orchestrator::{
        detector_processing::content,
        unary::{chunk, detect_content},
        Chunk, UNSUITABLE_INPUT_MESSAGE,
    },
};

/// Internal structure to capture chat messages (both request and response)
/// and prepare it for processing
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessageInternal {
    /// Index of the message
    pub message_index: usize,
    /// The role of the messages author.
    pub role: Role,
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
impl From<&ChatCompletionsRequest> for Vec<ChatMessageInternal> {
    fn from(value: &ChatCompletionsRequest) -> Self {
        value
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| ChatMessageInternal {
                message_index: index,
                role: message.role.clone(),
                content: message.content.clone(),
                refusal: message.refusal.clone(),
            })
            .collect()
    }
}

// Get Vec<ChatMessageInternal> from ChatCompletion
impl From<&Box<ChatCompletion>> for Vec<ChatMessageInternal> {
    fn from(value: &Box<ChatCompletion>) -> Self {
        value
            .choices
            .iter()
            .map(|choice| ChatMessageInternal {
                message_index: choice.index,
                role: choice.message.role.clone(),
                content: Some(Content::Text(
                    choice.message.content.clone().unwrap_or_default(),
                )),
                refusal: choice.message.refusal.clone(),
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

        let task_handle = tokio::spawn(async move {
            // Convert the request into a format that can be used for processing
            let chat_messages = Vec::<ChatMessageInternal>::from(&task.request);
            let detectors = task.request.detectors.clone().unwrap_or_default();

            let input_detections = match detectors.input {
                Some(detectors) if !detectors.is_empty() => {
                    // Call out to input detectors using chunk
                    message_detection(&ctx, &detectors, chat_messages, &task.headers).await?
                }
                _ => None,
            };

            debug!(?input_detections);

            if let Some(input_detections) = input_detections {
                let detections = sort_detections(input_detections);

                Ok(ChatCompletionsResponse::Unary(Box::new(ChatCompletion {
                    id: Uuid::new_v4().simple().to_string(),
                    model: task.request.model.clone(),
                    choices: vec![],
                    created: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64,
                    detections: Some(ChatDetections {
                        input: detections
                            .into_iter()
                            .map(|detection_result| InputDetectionResult {
                                message_index: detection_result.index,
                                results: detection_result.results,
                            })
                            .collect(),
                        output: vec![],
                    }),
                    warnings: vec![OrchestratorWarning::new(
                        DetectionWarningReason::UnsuitableInput,
                        UNSUITABLE_INPUT_MESSAGE,
                    )],
                    ..Default::default()
                })))
            } else {
                let client = ctx
                    .clients
                    .get_as::<OpenAiClient>("chat_generation")
                    .expect("chat_generation client not found");
                let mut chat_request = task.request;
                let model_id = chat_request.model.clone();
                // Remove detectors as chat completion server would reject extra parameter
                chat_request.detectors = None;
                let headers = task.headers.clone();
                let chat_completions = client
                    .chat_completions(chat_request, headers)
                    .await
                    .map_err(|error| Error::ChatGenerateRequestFailed {
                        id: model_id.clone(),
                        error,
                    })?;

                match handle_output_detections(
                    &chat_completions,
                    detectors.output,
                    ctx,
                    &task.headers,
                    model_id,
                )
                .await
                {
                    Some(chat_completion_detections) => Ok(chat_completion_detections),
                    None => Ok(chat_completions),
                }
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
// pub async fn input_detection(
pub async fn message_detection(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    chat_messages: Vec<ChatMessageInternal>,
    headers: &HeaderMap,
) -> Result<Option<Vec<DetectionResult>>, Error> {
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
                        .map(|(index, chunks)| {
                            let ctx = ctx.clone();
                            let detector_id = detector_id.clone();
                            let detector_params = detector_params.clone();
                            let headers = headers.clone();

                            tokio::spawn({
                                async move {
                                    // Call content detector on the chunks of particular message
                                    // and return the index and detection results
                                    let detections = detect_content(
                                        ctx.clone(),
                                        detector_id.clone(),
                                        default_threshold,
                                        detector_params.clone(),
                                        chunks,
                                        headers.clone(),
                                    )
                                    .await?;
                                    Ok((index, detections))
                                }
                            })
                        })
                        .collect::<Vec<_>>()
                }
                _ => unimplemented!(),
            }
        })
        .collect::<Vec<_>>();

    // Await detections
    let detections = try_join_all(tasks)
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, Error>>()?;

    // Build detection map
    let mut detection_map: BTreeMap<usize, Vec<ContentAnalysisResponse>> = BTreeMap::new();
    for (index, detections) in detections {
        if !detections.is_empty() {
            match detection_map.entry(index) {
                btree_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().extend_from_slice(&detections);
                }
                btree_map::Entry::Vacant(entry) => {
                    entry.insert(detections);
                }
            }
        }
    }

    // Build vec of DetectionResult
    // NOTE: seems unnecessary, could we just use the BTreeMap instead?
    let detections = detection_map
        .into_iter()
        .map(|(index, results)| DetectionResult { index, results })
        .collect::<Vec<_>>();

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
                    DetectorType::TextContents => content::filter_chat_messages(&messages),
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

fn sort_detections(mut detections: Vec<DetectionResult>) -> Vec<DetectionResult> {
    // Sort input detections by message_index
    detections.sort_by_key(|value| value.index);

    detections
        .into_iter()
        .map(|mut detection| {
            // sort detection by starting span
            detection.results.sort_by_key(|value| value.start);
            detection
        })
        .collect::<Vec<_>>()
}

async fn handle_output_detections(
    chat_completions: &ChatCompletionsResponse,
    detector_output: Option<HashMap<String, DetectorParams>>,
    ctx: Arc<Context>,
    headers: &HeaderMap,
    model_id: String,
) -> Option<ChatCompletionsResponse> {
    if let ChatCompletionsResponse::Unary(ref chat_completion) = chat_completions {
        let choices = Vec::<ChatMessageInternal>::from(chat_completion);

        let output_detections = match detector_output {
            Some(detectors) if !detectors.is_empty() => {
                let tasks = choices.into_iter().map(|choice| {
                    tokio::spawn({
                        let ctx = ctx.clone();
                        let detectors = detectors.clone();
                        let headers = headers.clone();
                        async move {
                            let result =
                                message_detection(&ctx, &detectors, vec![choice], &headers).await;

                            if let Ok(Some(detection_results)) = result {
                                return detection_results;
                            }

                            vec![]
                        }
                    })
                });

                let detections = try_join_all(tasks).await;

                match detections {
                    Ok(d) => Some(
                        d.iter()
                            .flatten()
                            .cloned()
                            .collect::<Vec<DetectionResult>>(),
                    ),
                    Err(_) => None,
                }
            }
            _ => None,
        };

        debug!(?output_detections);

        match output_detections {
            Some(output_detections) if !output_detections.is_empty() => {
                let detections = sort_detections(output_detections);

                return Some(ChatCompletionsResponse::Unary(Box::new(ChatCompletion {
                    id: Uuid::new_v4().simple().to_string(),
                    object: chat_completion.object.clone(),
                    created: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64,
                    model: model_id.to_string(),
                    choices: chat_completion.choices.clone(),
                    usage: chat_completion.usage.clone(),
                    system_fingerprint: chat_completion.system_fingerprint.clone(),
                    service_tier: chat_completion.service_tier.clone(),
                    detections: Some(ChatDetections {
                        input: vec![],
                        output: detections
                            .into_iter()
                            .map(|detection_result| OutputDetectionResult {
                                choice_index: detection_result.index,
                                results: detection_result.results,
                            })
                            .collect(),
                    }),
                    warnings: vec![OrchestratorWarning::new(
                        DetectionWarningReason::UnsuitableOutput,
                        UNSUITABLE_OUTPUT_MESSAGE,
                    )],
                })));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::any::{Any, TypeId};

    use super::*;
    use crate::{
        config::DetectorConfig,
        orchestrator::{ClientMap, OrchestratorConfig},
    };

    // Test to verify preprocess_chat_messages works correctly for multiple content type detectors
    // with single message in chat request
    #[test]
    fn pretest_process_chat_messages_multiple_content_detector() {
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
            role: Role::Assistant,
            ..Default::default()
        }];
        let processed_messages = preprocess_chat_messages(&ctx, &detectors, messages).unwrap();
        // Assertions
        assert!(processed_messages[detector_1_id].len() == 1);
        assert!(processed_messages[detector_2_id].len() == 1);
    }

    // Test preprocess_chat_messages returns error correctly for multiple content type detectors
    // with incorrect message requirements
    #[test]
    fn pretest_process_chat_messages_error_handling() {
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
            role: Role::Tool,
            ..Default::default()
        }];

        let processed_messages = preprocess_chat_messages(&ctx, &detectors, messages);

        // Assertions
        assert!(processed_messages.is_err());
        let error = processed_messages.unwrap_err();
        assert_eq!(error.type_id(), TypeId::of::<Error>());
        assert_eq!(
            error.to_string(),
            "validation error: Last message role must be user, assistant, or system"
        );
    }
    // validate chat completions request with invalid fields
    // (nonexistant fields or typos)
    #[test]
    fn test_validate() {
        // Additional unknown field (additional_field)
        let json_data = r#"
        {
           "messages": [
            {
                "content": "this is a nice sentence",
                "role": "user",
                "name": "string"
            }
            ],
            "model": "my_model",
            "additional_field": "test",
            "n": 1,
            "temperature": 1,
            "top_p": 1,
            "user": "user-1234",
            "detectors": {
                "input": {}
            }
        }
        "#;
        let result: Result<ChatCompletionsRequest, _> = serde_json::from_str(json_data);
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error
            .to_string()
            .contains("unknown field `additional_field"));

        // Additional unknown field (additional_message")
        let json_data = r#"
        {
           "messages": [
            {
                "content": "this is a nice sentence",
                "role": "user",
                "name": "string",
                "additional_msg: "test"
            }
            ],
            "model": "my_model",
            "n": 1,
            "temperature": 1,
            "top_p": 1,
            "user": "user-1234",
            "detectors": {
                "input": {}
            }
        }
        "#;
        let result: Result<ChatCompletionsRequest, _> = serde_json::from_str(json_data);
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.to_string().contains("unknown field `additional_msg"));

        // Additional unknown field (typo for input field in detectors)
        let json_data = r#"
         {
            "messages": [
             {
                 "content": "this is a nice sentence",
                 "role": "user",
                 "name": "string"
             }
             ],
             "model": "my_model",
             "n": 1,
             "temperature": 1,
             "top_p": 1,
             "user": "user-1234",
             "detectors": {
                 "inputs": {}
             }
         }
         "#;
        let result: Result<ChatCompletionsRequest, _> = serde_json::from_str(json_data);
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.to_string().contains("unknown field `inputs"));
    }
}
