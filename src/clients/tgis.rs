use std::{collections::HashMap, pin::Pin};

use futures::{Stream, StreamExt};
use ginepro::LoadBalancedChannel;

use super::{create_grpc_clients, Error};
use crate::{
    clients::COMMON_ROUTER_KEY,
    config::ServiceConfig,
    pb::fmaas::{
        generation_service_client::GenerationServiceClient, BatchedGenerationRequest,
        BatchedGenerationResponse, BatchedTokenizeRequest, BatchedTokenizeResponse,
        GenerationResponse, ModelInfoRequest, ModelInfoResponse, SingleGenerationRequest,
    },
};

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct TgisClient {
    clients: HashMap<String, GenerationServiceClient<LoadBalancedChannel>>,
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
    ) -> Result<Pin<Box<dyn Stream<Item = GenerationResponse> + Send>>, Error> {
        let model_id = request.model_id.as_str();
        let response_stream = self
            .client(model_id)?
            .generate_stream(request)
            .await?
            .into_inner()
            .map(|resp| resp.unwrap())
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
