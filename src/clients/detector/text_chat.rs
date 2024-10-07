use async_trait::async_trait;

use crate::{
    clients::{Client, HttpClient},
    health::HealthCheckResult,
};

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TextChatDetectorClient {
    client: HttpClient,
}

#[cfg_attr(test, faux::methods)]
impl TextChatDetectorClient {
    pub fn new(client: HttpClient) -> Self {
        Self { client }
    }

    pub async fn text_chat(&self) {
        let _url = self.client.base_url().join("/api/v1/text/chat").unwrap();
        todo!()
    }
}

#[cfg_attr(test, faux::methods)]
#[async_trait]
impl Client for TextChatDetectorClient {
    fn name(&self) -> &str {
        "text_chat_detector"
    }

    async fn health(&self) -> HealthCheckResult {
        self.client.health().await
    }
}
