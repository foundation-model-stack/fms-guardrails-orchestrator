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

use async_trait::async_trait;

use super::DEFAULT_PORT;
use crate::{
    clients::{create_http_client, Client, HttpClient},
    config::ServiceConfig,
    health::HealthCheckResult,
};

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TextChatDetectorClient {
    client: HttpClient,
    health_client: Option<HttpClient>,
}

#[cfg_attr(test, faux::methods)]
impl TextChatDetectorClient {
    pub async fn new(config: &ServiceConfig, health_config: Option<&ServiceConfig>) -> Self {
        let client = create_http_client(DEFAULT_PORT, config).await;
        let health_client = if let Some(health_config) = health_config {
            Some(create_http_client(DEFAULT_PORT, health_config).await)
        } else {
            None
        };
        Self {
            client,
            health_client,
        }
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
        if let Some(health_client) = &self.health_client {
            health_client.health().await
        } else {
            self.client.health().await
        }
    }
}
