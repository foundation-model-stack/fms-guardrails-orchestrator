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

use super::{create_grpc_clients, BoxStream, Error, HealthProbe};
use crate::{
    clients::COMMON_ROUTER_KEY,
    config::ServiceConfig,
    orchestrator::HealthStatus,
    pb::{
        fmaas::{
            generation_service_client::GenerationServiceClient, BatchedGenerationRequest,
            BatchedGenerationResponse, BatchedTokenizeRequest, BatchedTokenizeResponse,
            GenerationResponse, ModelInfoRequest, ModelInfoResponse, SingleGenerationRequest,
        },
        grpc::health::v1::{health_client::HealthClient, HealthCheckRequest},
    },
};

#[cfg_attr(test, faux::create, derive(Default))]
#[derive(Clone)]
pub struct TgisClient {
    clients: HashMap<String, GenerationServiceClient<LoadBalancedChannel>>,
    health_clients: HashMap<String, HealthClient<LoadBalancedChannel>>,
}

impl HealthProbe for TgisClient {
    async fn ready(&self) -> Result<HashMap<String, HealthStatus>, Error> {
        let mut results = HashMap::new();
        for (model_id, mut client) in self.health_clients() {
            let response = client
                .check(HealthCheckRequest {
                    service: model_id.clone(),
                })
                .await;
            let status = response.map(|_| HealthStatus::Ready).unwrap_or_else(|e| {
                if e.code() == tonic::Code::Unknown {
                    HealthStatus::Unknown
                } else {
                    Error::from(e).status_code().into()
                }
            });
            results.insert(model_id, status);
        }
        Ok(results)
    }
}

#[cfg_attr(test, faux::methods)]
impl TgisClient {
    pub async fn new(default_port: u16, config: &[(String, ServiceConfig)]) -> Self {
        let clients = create_grpc_clients(default_port, config, GenerationServiceClient::new).await;
        let health_clients = create_grpc_clients(default_port, config, HealthClient::new).await;
        Self {
            clients,
            health_clients,
        }
    }

    fn client(
        &self,
        _model_id: &str,
    ) -> Result<GenerationServiceClient<LoadBalancedChannel>, Error> {
        // NOTE: We currently forward requests to the common-router, so we use a single client.
        let model_id = COMMON_ROUTER_KEY;
        Ok(self
            .clients
            .get(model_id)
            .ok_or_else(|| Error::ModelNotFound {
                model_id: model_id.to_string(),
            })?
            .clone())
    }

    fn health_clients(&self) -> HashMap<String, HealthClient<LoadBalancedChannel>> {
        self.health_clients.clone()
    }

    pub async fn generate(
        &self,
        request: BatchedGenerationRequest,
    ) -> Result<BatchedGenerationResponse, Error> {
        let model_id = request.model_id.as_str();
        Ok(self.client(model_id)?.generate(request).await?.into_inner())
    }

    pub async fn generate_stream(
        &self,
        request: SingleGenerationRequest,
    ) -> Result<BoxStream<Result<GenerationResponse, Error>>, Error> {
        let model_id = request.model_id.as_str();
        let response_stream = self
            .client(model_id)?
            .generate_stream(request)
            .await?
            .into_inner()
            .map_err(Into::into)
            .boxed();
        Ok(response_stream)
    }

    pub async fn tokenize(
        &self,
        request: BatchedTokenizeRequest,
    ) -> Result<BatchedTokenizeResponse, Error> {
        let model_id = request.model_id.as_str();
        Ok(self.client(model_id)?.tokenize(request).await?.into_inner())
    }

    pub async fn model_info(&self, request: ModelInfoRequest) -> Result<ModelInfoResponse, Error> {
        let model_id = request.model_id.as_str();
        Ok(self
            .client(model_id)?
            .model_info(request)
            .await?
            .into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pb::fmaas::model_info_response;

    #[tokio::test]
    async fn test_model_info() {
        // Initialize a mock object from `TgisClient`
        let mut mock_client = TgisClient::faux();

        let request = ModelInfoRequest {
            model_id: "test-model-1".to_string(),
        };

        let expected_response = ModelInfoResponse {
            max_sequence_length: 2,
            max_new_tokens: 20,
            max_beam_width: 3,
            model_kind: model_info_response::ModelKind::DecoderOnly.into(),
            max_beam_sequence_lengths: [].to_vec(),
        };
        // Construct a behavior for the mock object
        faux::when!(mock_client.model_info(request.clone()))
            .once()
            .then_return(Ok(expected_response.clone()));
        // Test the mock object's behaviour
        assert_eq!(
            mock_client.model_info(request).await.unwrap(),
            expected_response
        );
    }
}
