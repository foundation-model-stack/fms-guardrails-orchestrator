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
use tonic::Request;
use tracing::instrument;

use super::{
    create_grpc_client, grpc::GrpcClient, grpc_request_with_headers, BoxStream, Client, Error,
};
use crate::{
    config::ServiceConfig,
    grpc_call, grpc_stream_call,
    health::{HealthCheckResult, HealthStatus},
    pb::{
        caikit::runtime::nlp::{
            nlp_service_client::NlpServiceClient, nlp_service_server,
            ServerStreamingTextGenerationTaskRequest, TextGenerationTaskRequest,
            TokenClassificationTaskRequest, TokenizationTaskRequest,
        },
        caikit_data_model::nlp::{
            GeneratedTextResult, GeneratedTextStreamResult, TokenClassificationResults,
            TokenizationResults,
        },
        grpc::health::v1::{health_client::HealthClient, health_server, HealthCheckRequest},
    },
};

const DEFAULT_PORT: u16 = 8085;
const MODEL_ID_HEADER_NAME: &str = "mm-model-id";

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct NlpClient {
    client: GrpcClient<NlpServiceClient<LoadBalancedChannel>>,
    health_client: GrpcClient<HealthClient<LoadBalancedChannel>>,
}

#[cfg_attr(test, faux::methods)]
impl NlpClient {
    pub async fn new(config: &ServiceConfig) -> Self {
        let client = create_grpc_client(
            nlp_service_server::SERVICE_NAME,
            DEFAULT_PORT,
            config,
            NlpServiceClient::new,
            true,
        )
        .await;
        let health_client = create_grpc_client(
            health_server::SERVICE_NAME,
            DEFAULT_PORT,
            config,
            HealthClient::new,
            false,
        )
        .await;
        Self {
            client,
            health_client,
        }
    }

    #[instrument(skip_all, fields(model_id))]
    pub async fn tokenization_task_predict(
        &self,
        model_id: &str,
        request: TokenizationTaskRequest,
        headers: HeaderMap,
    ) -> Result<TokenizationResults, Error> {
        let request = request_with_headers(request, model_id, headers);
        grpc_call!(
            self.client,
            request,
            NlpServiceClient::tokenization_task_predict
        )
    }

    #[instrument(skip_all, fields(model_id))]
    pub async fn token_classification_task_predict(
        &self,
        model_id: &str,
        request: TokenClassificationTaskRequest,
        headers: HeaderMap,
    ) -> Result<TokenClassificationResults, Error> {
        let request = request_with_headers(request, model_id, headers);
        grpc_call!(
            self.client,
            request,
            NlpServiceClient::token_classification_task_predict
        )
    }

    #[instrument(skip_all, fields(model_id))]
    pub async fn text_generation_task_predict(
        &self,
        model_id: &str,
        request: TextGenerationTaskRequest,
        headers: HeaderMap,
    ) -> Result<GeneratedTextResult, Error> {
        let request = request_with_headers(request, model_id, headers);
        grpc_call!(
            self.client,
            request,
            NlpServiceClient::text_generation_task_predict
        )
    }

    #[instrument(skip_all, fields(model_id))]
    pub async fn server_streaming_text_generation_task_predict(
        &self,
        model_id: &str,
        request: ServerStreamingTextGenerationTaskRequest,
        headers: HeaderMap,
    ) -> Result<BoxStream<Result<GeneratedTextStreamResult, Error>>, Error> {
        let request = request_with_headers(request, model_id, headers);
        grpc_stream_call!(
            self.client,
            request,
            NlpServiceClient::server_streaming_text_generation_task_predict
        )
    }
}

#[cfg_attr(test, faux::methods)]
#[async_trait]
impl Client for NlpClient {
    fn name(&self) -> &str {
        "nlp"
    }

    async fn health(&self) -> Result<HealthCheckResult, Error> {
        let request =
            grpc_request_with_headers(HealthCheckRequest { service: "".into() }, HeaderMap::new());
        let response = async { grpc_call!(self.health_client, request, HealthClient::check) }.await;
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

/// Turns an NLP client gRPC request body of type `T` and headers into a `tonic::Request<T>`.
/// Also injects provided `model_id` and `traceparent` from current context into headers.
fn request_with_headers<T>(request: T, model_id: &str, headers: HeaderMap) -> Request<T> {
    let mut request = grpc_request_with_headers(request, headers);
    request
        .metadata_mut()
        .insert(MODEL_ID_HEADER_NAME, model_id.parse().unwrap());
    request
}
