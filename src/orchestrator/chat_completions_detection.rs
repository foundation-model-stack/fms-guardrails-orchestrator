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

use tracing::{info, instrument};

use super::{ChatCompletionsDetectionTask, Error, Orchestrator};
use crate::clients::openai::{
    ChatCompletionChoice, ChatCompletionsRequest, ChatCompletionsResponse, Content, OpenAiClient,
};
use serde::{Deserialize, Serialize};

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
        let client = self
            .ctx
            .clients
            .get_as::<OpenAiClient>("chat_generation")
            .expect("chat_generation client not found");
        Ok(client.chat_completions(task.request, task.headers).await?)
    }
}
