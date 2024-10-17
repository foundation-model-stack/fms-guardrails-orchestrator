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
use axum::http::HeaderMap;
use futures::{StreamExt, TryStreamExt};
use ginepro::LoadBalancedChannel;
use tonic::Code;

use super::{create_grpc_client, errors::grpc_to_http_code, BoxStream, Client, Error};
use crate::{
    config::ServiceConfig,
    health::{HealthCheckResult, HealthStatus},
    pb::fmaas::{
        generation_service_client::GenerationServiceClient, BatchedGenerationRequest,
        BatchedGenerationResponse, BatchedTokenizeRequest, BatchedTokenizeResponse,
        GenerationResponse, ModelInfoRequest, ModelInfoResponse, SingleGenerationRequest,
    },
};

const DEFAULT_PORT: u16 = 8033;

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TgisClient {
    client: GenerationServiceClient<LoadBalancedChannel>,
}

#[cfg_attr(test, faux::methods)]
impl TgisClient {
    pub async fn new(config: &ServiceConfig) -> Self {
        let client = create_grpc_client(DEFAULT_PORT, config, GenerationServiceClient::new).await;
        Self { client }
    }

    pub async fn generate(
        &self,
        request: BatchedGenerationRequest,
        _headers: HeaderMap,
    ) -> Result<BatchedGenerationResponse, Error> {
        let mut client = self.client.clone();
        Ok(client.generate(request).await?.into_inner())
    }

    pub async fn generate_stream(
        &self,
        request: SingleGenerationRequest,
        _headers: HeaderMap,
    ) -> Result<BoxStream<Result<GenerationResponse, Error>>, Error> {
        let mut client = self.client.clone();
        let response_stream = client
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
        _headers: HeaderMap,
    ) -> Result<BatchedTokenizeResponse, Error> {
        let mut client = self.client.clone();
        Ok(client.tokenize(request).await?.into_inner())
    }

    pub async fn model_info(&self, request: ModelInfoRequest) -> Result<ModelInfoResponse, Error> {
        let mut client = self.client.clone();
        Ok(client.model_info(request).await?.into_inner())
    }
}

#[cfg_attr(test, faux::methods)]
#[async_trait]
impl Client for TgisClient {
    fn name(&self) -> &str {
        "tgis"
    }

    async fn health(&self) -> HealthCheckResult {
        let mut client = self.client.clone();
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
        let status = if matches!(code, Code::Ok) {
            HealthStatus::Healthy
        } else {
            HealthStatus::Unhealthy
        };
        HealthCheckResult {
            status,
            code: grpc_to_http_code(code),
            reason: None,
        }
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
