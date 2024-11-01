use tracing::{info, instrument};

use super::{ChatCompletionTask, Error, Orchestrator};
use crate::clients::openai::{ChatCompletionResponse, OpenAiClient};

impl Orchestrator {
    #[instrument(skip_all, fields(trace_id = ?task.trace_id, headers = ?task.headers))]
    pub async fn handle_chat_completions(
        &self,
        task: ChatCompletionTask,
    ) -> Result<ChatCompletionResponse, Error> {
        info!("handling chat completion task");
        let client = self
            .ctx
            .clients
            .get_as::<OpenAiClient>("chat_generation")
            .expect("chat_generation client not found");
        Ok(client.chat_completions(task.request, task.headers).await?)
    }
}
