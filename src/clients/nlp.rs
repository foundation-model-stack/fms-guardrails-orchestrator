use std::collections::HashMap;

use futures::StreamExt;
use ginepro::LoadBalancedChannel;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Request;

use super::{create_grpc_clients, Error};
use crate::{
    clients::COMMON_ROUTER_KEY,
    config::ServiceConfig,
    pb::{
        caikit::runtime::nlp::{
            nlp_service_client::NlpServiceClient, ServerStreamingTextGenerationTaskRequest,
            TextGenerationTaskRequest, TokenClassificationTaskRequest, TokenizationTaskRequest,
        },
        caikit_data_model::nlp::{
            GeneratedTextResult, GeneratedTextStreamResult, TokenClassificationResults,
            TokenizationResults,
        },
    },
};

const MODEL_ID_HEADER_NAME: &str = "mm-model-id";

#[derive(Clone)]
pub struct NlpClient {
    clients: HashMap<String, NlpServiceClient<LoadBalancedChannel>>,
}

impl NlpClient {
    pub async fn new(default_port: u16, config: &[(String, ServiceConfig)]) -> Self {
        let clients = create_grpc_clients(default_port, config, NlpServiceClient::new).await;
        Self { clients }
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
    ) -> Result<ReceiverStream<GeneratedTextStreamResult>, Error> {
        let request = request_with_model_id(request, model_id);
        let mut response_stream = self
            .client(model_id)?
            .server_streaming_text_generation_task_predict(request)
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
}

fn request_with_model_id<T>(request: T, model_id: &str) -> Request<T> {
    let mut request = Request::new(request);
    request
        .metadata_mut()
        .insert(MODEL_ID_HEADER_NAME, model_id.parse().unwrap());
    request
}
