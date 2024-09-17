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
use tonic::Code;

use super::{create_grpc_clients, BoxStream, ClientCode, Error};
use crate::{
    clients::COMMON_ROUTER_KEY,
    config::ServiceConfig,
    health::{HealthCheckResult, HealthProbe, HealthStatus},
    pb::fmaas::{
        generation_service_client::GenerationServiceClient, BatchedGenerationRequest,
        BatchedGenerationResponse, BatchedTokenizeRequest, BatchedTokenizeResponse,
        GenerationResponse, ModelInfoRequest, ModelInfoResponse, SingleGenerationRequest,
    },
};

#[cfg_attr(test, faux::create, derive(Default))]
#[derive(Clone)]
pub struct TgisClient {
    clients: HashMap<String, GenerationServiceClient<LoadBalancedChannel>>,
}

#[cfg_attr(test, faux::methods)]
impl HealthProbe for TgisClient {
    async fn health(&self) -> Result<HashMap<String, HealthCheckResult>, Error> {
        let mut results = HashMap::with_capacity(self.clients.len());
        for (model_id, mut client) in self.clients.clone() {
            let response = client
                .model_info(ModelInfoRequest {
                    model_id: "".into(),
                })
                .await;
            let code = match response {
                Ok(_) => Code::Ok,
                Err(status) if matches!(status.code(), Code::InvalidArgument | Code::NotFound) => {
                    Code::Ok
                }
                Err(status) => status.code(),
            };
            let health_status = if matches!(code, Code::Ok) {
                HealthStatus::Ready
            } else {
                HealthStatus::NotReady
            };
            results.insert(
                model_id,
                HealthCheckResult {
                    health_status,
                    response_code: ClientCode::Grpc(code),
                    reason: None,
                },
            );
        }
        Ok(results)
    }
}

#[cfg_attr(test, faux::methods)]
impl TgisClient {
    pub async fn new(default_port: u16, config: &[(String, ServiceConfig)]) -> Self {
        let clients = create_grpc_clients(default_port, config, GenerationServiceClient::new).await;
        Self { clients }
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
