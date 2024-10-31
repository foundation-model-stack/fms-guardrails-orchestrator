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
use axum::http::{HeaderMap, StatusCode};
use futures::{StreamExt, TryStreamExt};
use ginepro::LoadBalancedChannel;
use tracing::instrument;

use super::{
    create_grpc_client, grpc::GrpcClient, grpc_request_with_headers, BoxStream, Client, Error,
};
use crate::{
    config::ServiceConfig,
    grpc_call, grpc_stream_call,
    health::{HealthCheckResult, HealthStatus},
    pb::fmaas::{
        generation_service_client::GenerationServiceClient, generation_service_server,
        BatchedGenerationRequest, BatchedGenerationResponse, BatchedTokenizeRequest,
        BatchedTokenizeResponse, GenerationResponse, ModelInfoRequest, ModelInfoResponse,
        SingleGenerationRequest,
    },
};

const DEFAULT_PORT: u16 = 8033;

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TgisClient {
    client: GrpcClient<GenerationServiceClient<LoadBalancedChannel>>,
}

#[cfg_attr(test, faux::methods)]
impl TgisClient {
    pub async fn new(config: &ServiceConfig) -> Self {
        let client = create_grpc_client(
            generation_service_server::SERVICE_NAME,
            DEFAULT_PORT,
            config,
            GenerationServiceClient::new,
            true,
        )
        .await;
        Self { client }
    }

    #[instrument(skip_all, fields(model_id = request.model_id))]
    pub async fn generate(
        &self,
        request: BatchedGenerationRequest,
        headers: HeaderMap,
    ) -> Result<BatchedGenerationResponse, Error> {
        let request = grpc_request_with_headers(request, headers);
        grpc_call!(self.client, request, GenerationServiceClient::generate)
    }

    #[instrument(skip_all, fields(model_id = request.model_id))]
    pub async fn generate_stream(
        &self,
        request: SingleGenerationRequest,
        headers: HeaderMap,
    ) -> Result<BoxStream<Result<GenerationResponse, Error>>, Error> {
        let request = grpc_request_with_headers(request, headers);
        grpc_stream_call!(
            self.client,
            request,
            GenerationServiceClient::generate_stream
        )
    }

    #[instrument(skip_all, fields(model_id = request.model_id))]
    pub async fn tokenize(
        &self,
        request: BatchedTokenizeRequest,
        headers: HeaderMap,
    ) -> Result<BatchedTokenizeResponse, Error> {
        let request = grpc_request_with_headers(request, headers);
        grpc_call!(self.client, request, GenerationServiceClient::tokenize)
    }

    pub async fn model_info(&self, request: ModelInfoRequest) -> Result<ModelInfoResponse, Error> {
        let request = grpc_request_with_headers(request, HeaderMap::new());
        grpc_call!(self.client, request, GenerationServiceClient::model_info)
    }
}

#[cfg_attr(test, faux::methods)]
#[async_trait]
impl Client for TgisClient {
    fn name(&self) -> &str {
        "tgis"
    }

    async fn health(&self) -> Result<HealthCheckResult, Error> {
        let request = grpc_request_with_headers(
            ModelInfoRequest {
                model_id: "".into(),
            },
            HeaderMap::new(),
        );
        let response =
            async { grpc_call!(self.client, request, GenerationServiceClient::model_info) }.await;
        let code = match response {
            Ok(_) => StatusCode::OK,
            Err(error) => match error {
                Error::Grpc {
                    code: StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND,
                    ..
                } => StatusCode::OK,
                Error::Grpc { code, .. } => code,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
        };
        let status = if matches!(code, StatusCode::OK) {
            HealthStatus::Healthy
        } else {
            HealthStatus::Unhealthy
        };
        Ok(HealthCheckResult {
            status,
            code,
            reason: None,
        })
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
