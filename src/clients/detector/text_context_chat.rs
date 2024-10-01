use async_trait::async_trait;

use crate::{
    clients::{Client, HttpClient},
    health::HealthCheckResult,
};

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TextContextChatDetectorClient {
    client: HttpClient,
}

#[cfg_attr(test, faux::methods)]
impl TextContextChatDetectorClient {
    pub fn new(client: HttpClient) -> Self {
        Self { client }
    }

    pub async fn text_context_chat(&self) {
        let _url = self
            .client
            .base_url()
            .join("/api/v1/text/context/chat")
            .unwrap();
        todo!()
    }
}

#[cfg_attr(test, faux::methods)]
#[async_trait]
impl Client for TextContextChatDetectorClient {
    fn name(&self) -> &str {
        "text_context_chat_detector"
    }

    async fn health(&self) -> HealthCheckResult {
        self.client.health().await
    }
}
