use std::collections::HashMap;

use futures::StreamExt;
use ginepro::LoadBalancedChannel;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{create_grpc_clients, Error};
use crate::{
    config::ServiceConfig,
    pb::fmaas::{
        generation_service_client::GenerationServiceClient, BatchedGenerationRequest,
        BatchedGenerationResponse, BatchedTokenizeRequest, BatchedTokenizeResponse,
        GenerationResponse, ModelInfoRequest, ModelInfoResponse, SingleGenerationRequest,
    },
};

#[derive(Clone)]
pub struct TgisClient {
    clients: HashMap<String, GenerationServiceClient<LoadBalancedChannel>>,
}

impl TgisClient {
    pub async fn new(default_port: u16, config: &[(String, ServiceConfig)]) -> Self {
        let clients = create_grpc_clients(default_port, config, GenerationServiceClient::new).await;
        Self { clients }
    }

    fn client(
        &self,
        _model_id: &str,
    ) -> Result<GenerationServiceClient<LoadBalancedChannel>, Error> {
        // NOTE: We currently forward requests to the tgis-router, so we use a single client.
        let model_id = "tgis-router";
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
    ) -> Result<ReceiverStream<GenerationResponse>, Error> {
        let model_id = request.model_id.as_str();
        let mut response_stream = self
            .client(model_id)?
            .generate_stream(request)
            .await?
            .into_inner();
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(async move {
            while let Some(Ok(message)) = response_stream.next().await {
                let _ = tx.send(message).await;
            }
        });
        Ok(ReceiverStream::new(rx))
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
