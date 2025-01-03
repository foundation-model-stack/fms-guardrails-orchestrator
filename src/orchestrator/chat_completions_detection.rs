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
use std::{borrow::BorrowMut, future::IntoFuture, ops::Deref, pin::Pin};
use axum::http::HeaderMap;
use futures::{future::{try_join_all}, Future};
use std::{collections::HashMap, sync::Arc};
use tracing::{debug, info, instrument};
use tokio::sync::Mutex;

use super::{get_chunker_ids, ChatCompletionsDetectionTask, Context, Error, Orchestrator};
use crate::{
    clients::{
        detector::{self, ChatDetectionRequest, ContentAnalysisRequest, ContentAnalysisResponse},
        openai::{
            ChatCompletionChoice, ChatCompletionsRequest, ChatCompletionsResponse, Content,
            InputDetectionResult, OpenAiClient,
        },
    },
    config::DetectorType,
    models::DetectorParams,
    orchestrator::{
        detector_processing::content, unary::chunk, Chunk
    },
};
use serde::{Deserialize, Serialize};


// pub type ChunkChatMessagesResult = Box<dyn Future<Output = ChatMessagesInternal> + Send>;
// pub type ChunkChatMessagesResult = Pin<dyn Future<Output = ChatMessagesInternal> + Send>;
pub type ChunkResult<T> = Pin<Box<dyn Future<Output = T> + Send>>;


/// Internal structure to capture chat messages (both request and response)
/// and prepare it for processing
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ChatMessageInternal {
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
        value.messages.iter().for_each(|m| {
            messages.push({
                ChatMessageInternal {
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
            let input_detectors = request.detectors.input;

            let input_detections = match input_detectors {
                Some(detectors) if !detectors.is_empty() => {
                    
                    // Call out to input detectors using chunk
                    input_detection(&ctx, &detectors, chat_messages, headers.clone()).await.unwrap()
                }
                _ => None,
            };
        });

        let client = self
            .ctx
            .clients
            .get_as::<OpenAiClient>("chat_generation")
            .expect("chat_generation client not found");
        Ok(client.chat_completions(task.request, task.headers).await?)
    }
}

#[instrument(skip_all)]
pub async fn input_detection(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    // chunks: Arc<HashMap<String, ChunkResult<ChatMessagesInternal>>>,
    // chunks: HashMap<String, Arc<ChunkResult<ChatMessagesInternal>>>,
    chat_messages: ChatMessagesInternal,
    headers: HeaderMap,
) -> Result<Option<Vec<InputDetectionResult>>, Error> {
    debug!(?detectors, "starting input detection on chat completions");

    let ctx = ctx.clone();

    // filter chat messages based on individual detectors to prepare for chunking
    let filtered_chat_messages = filter_chat_messages(&ctx, &detectors, chat_messages)?;

    // Call out to the chunker to get chunks of messages based on detector type
    let chunks = detector_chunk_task(&ctx, filtered_chat_messages).await?;

    let tasks = detectors
        .iter()
        .map(|(detector_id, detector_params)| {
            let detector_id = detector_id.clone();
            let detector_params = detector_params.clone();
   
            let detector_config =
                ctx.config.detectors.get(&detector_id).unwrap_or_else(|| {
                    panic!("detector config not found for {}", detector_id)
                });
            let default_threshold = detector_config.default_threshold;

            let detector_type = &detector_config.r#type;

            let headers = headers.clone();

            let messages = chunks
                .get(&detector_id)
                .unwrap()
                // .unwrap_or_else(|| panic!("chunk not found for {}", chunker_id))
                .clone();
            
            let ctx = ctx.clone();
            async move {
                match detector_type {
                    DetectorType::TextContents => {
                        // call detection using curated chunks
                        tokio::spawn(async move {
                            detect_content(
                                ctx,
                                detector_id,
                                detector_params,
                                messages,
                                default_threshold,
                                headers.clone(),
                            )
                            .await
                        }).await
                    }
                    _ => unimplemented!(),
                }
            }
        })
        // .collect::<Result<Vec<_>, Error>>()?;
        .collect::<Vec<_>>();

        let results = try_join_all(tasks)
        .await?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    Ok(Some(results))
}


// Function to filter messages based on individual detectors
fn filter_chat_messages(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
    messages: Vec<ChatMessageInternal>,
) -> Result<HashMap<String, ChatMessagesInternal>, Error> {

    let chat_messages = detectors
    .iter()
    .map(|(detector_id, _)| {
        let ctx = ctx.clone();
        let detector_id = detector_id.clone();
        let detector_config =
            ctx.config.detectors.get(&detector_id).unwrap_or_else(|| {
                panic!("detector config not found for {}", detector_id)
            });
        let detector_type = &detector_config.r#type;
        // Filter messages based on detector type
        match detector_type {
            DetectorType::TextContents => {
                match content::filter_chat_message(messages.clone()) {
                    Ok(filtered_messages) => Ok((detector_id, filtered_messages)),
                    Err(e) => return Err(e),
                }
            }
            _ => unimplemented!(),
        }
    })
    .take_while(Result::is_ok)
    .map(Result::unwrap)
    .collect::<HashMap<String, ChatMessagesInternal>>();

    Ok(chat_messages)
}

// Function to chunk ChatMessagesInternal based on the chunker id and return chunks in ChatMessagesInternal form
async fn detector_chunk_task(
    ctx: &Arc<Context>,
    detector_chat_messages: HashMap<String, ChatMessagesInternal>) -> Result<HashMap<String, Vec<Chunk>>, Error> {

    // let chunking_tasks = Vec::new();
    let mut chunks = HashMap::<String, Vec<Chunk>>::new();

    // TODO: Improve error handling for the code below
    for (detector_id, chat_messages) in detector_chat_messages.iter() {

        let chunk_tasks = chat_messages
            .iter()
            .map(|message|{
                let text = match message.content.as_ref().unwrap() {
                    Content::Text(value) => value,
                    _ => panic!("Only text content accepted")
                };
                let offset: usize = 0;
                let task = tokio::spawn({
                    let detector_id = detector_id.clone();
                    let text = text.clone();
                    let ctx = ctx.clone();
                    async move {
                        let chunker_id = ctx
                            .config
                            .get_chunker_id(&detector_id)
                            .unwrap();
                        chunk(&ctx, chunker_id, offset, text).await
                    }
                });
                task
                // chunking_tasks.push((detector_id, task));
            })
            .collect::<Vec<_>>();
        
        let results = try_join_all(chunk_tasks)
            .await?
            .into_iter()
            .collect::<Result<Vec<_>, Error>>()?
            // .collect::<Vec<_>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        
        chunks.insert(detector_id.clone(), results);

    }

   Ok(chunks)

}

async fn detect_content(
    ctx: Arc<Context>,
    detector_id: String,
    detector_params: DetectorParams,
    chunk: Vec<Chunk>,
    // chunk_task: &Arc<Pin<Box<dyn Future<Output = Vec<ChatMessageInternal>> + Send>>>,
    default_threshold: f64,
    headers: HeaderMap,
) -> Result<InputDetectionResult, Error> {
    unimplemented!()
}
