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

use std::collections::HashMap;

use futures::{StreamExt, TryStreamExt};
use ginepro::LoadBalancedChannel;
use tonic::Request;

use super::{create_grpc_clients, BoxStream, Error};
use crate::{
    clients::COMMON_ROUTER_KEY,
    config::ServiceConfig,
    health::{HealthCheckResult, HealthProbe},
    pb::{
        caikit::runtime::nlp::{
            nlp_service_client::NlpServiceClient, ServerStreamingTextGenerationTaskRequest,
            TextGenerationTaskRequest, TokenClassificationTaskRequest, TokenizationTaskRequest,
        },
        caikit_data_model::nlp::{
            GeneratedTextResult, GeneratedTextStreamResult, TokenClassificationResults,
            TokenizationResults,
        },
        grpc::health::v1::{health_client::HealthClient, HealthCheckRequest},
    },
};

const MODEL_ID_HEADER_NAME: &str = "mm-model-id";

#[cfg_attr(any(test, feature = "mock"), faux::create, derive(Default))]
#[derive(Clone)]
pub struct NlpClient {
    clients: HashMap<String, NlpServiceClient<LoadBalancedChannel>>,
    health_clients: HashMap<String, HealthClient<LoadBalancedChannel>>,
}

#[cfg_attr(any(test, feature = "mock"), faux::methods)]
impl HealthProbe for NlpClient {
    async fn health(&self) -> Result<HashMap<String, HealthCheckResult>, Error> {
        let mut results = HashMap::with_capacity(self.health_clients.len());
        for (model_id, mut client) in self.health_clients.clone() {
            results.insert(
                model_id.clone(),
                client
                    .check(HealthCheckRequest {
                        service: model_id.clone(),
                    })
                    .await
                    .into(),
            );
        }
        Ok(results)
    }
}

#[cfg_attr(any(test, feature = "mock"), faux::methods)]
impl NlpClient {
    pub async fn new(default_port: u16, config: &[(String, ServiceConfig)]) -> Self {
        let clients = create_grpc_clients(default_port, config, NlpServiceClient::new).await;
        let health_clients = create_grpc_clients(default_port, config, HealthClient::new).await;
        Self {
            clients,
            health_clients,
        }
    }

    fn client(&self, _model_id: &str) -> Result<NlpServiceClient<LoadBalancedChannel>, Error> {
        // NOTE: We currently forward requests to common router, so we use a single client.
        let model_id = COMMON_ROUTER_KEY;
        Ok(self
            .clients
            .get(model_id)
            .ok_or_else(|| Error::ModelNotFound {
                model_id: model_id.to_string(),
            })?
            .clone())
    }

    pub async fn tokenization_task_predict(
        &self,
        model_id: &str,
        request: TokenizationTaskRequest,
    ) -> Result<TokenizationResults, Error> {
        let request = request_with_model_id(request, model_id);
        Ok(self
            .client(model_id)?
            .tokenization_task_predict(request)
            .await?
            .into_inner())
    }

    pub async fn token_classification_task_predict(
        &self,
        model_id: &str,
        request: TokenClassificationTaskRequest,
    ) -> Result<TokenClassificationResults, Error> {
        let request = request_with_model_id(request, model_id);
        Ok(self
            .client(model_id)?
            .token_classification_task_predict(request)
            .await?
            .into_inner())
    }

    pub async fn text_generation_task_predict(
        &self,
        model_id: &str,
        request: TextGenerationTaskRequest,
    ) -> Result<GeneratedTextResult, Error> {
        let request = request_with_model_id(request, model_id);
        Ok(self
            .client(model_id)?
            .text_generation_task_predict(request)
            .await?
            .into_inner())
    }

    pub async fn server_streaming_text_generation_task_predict(
        &self,
        model_id: &str,
        request: ServerStreamingTextGenerationTaskRequest,
    ) -> Result<BoxStream<Result<GeneratedTextStreamResult, Error>>, Error> {
        let request = request_with_model_id(request, model_id);
        let response_stream = self
            .client(model_id)?
            .server_streaming_text_generation_task_predict(request)
            .await?
            .into_inner()
            .map_err(Into::into)
            .boxed();
        Ok(response_stream)
    }
}

fn request_with_model_id<T>(request: T, model_id: &str) -> Request<T> {
    let mut request = Request::new(request);
    request
        .metadata_mut()
        .insert(MODEL_ID_HEADER_NAME, model_id.parse().unwrap());
    request
}
