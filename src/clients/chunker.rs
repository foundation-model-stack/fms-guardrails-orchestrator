use std::{collections::HashMap, pin::Pin};

use futures::{Future, Stream, StreamExt};
use ginepro::LoadBalancedChannel;
use tonic::{Request, Response, Status, Streaming};

use super::{create_grpc_clients, Error};
use crate::{
    config::ServiceConfig,
    pb::{
        caikit::runtime::chunkers::{
            chunkers_service_client::ChunkersServiceClient, BidiStreamingTokenizationTaskRequest,
            TokenizationTaskRequest,
        },
        caikit_data_model::nlp::{TokenizationResults, TokenizationStreamResult},
    },
};

const MODEL_ID_HEADER_NAME: &str = "mm-model-id";

type StreamingTokenizationResult = Result<Response<Streaming<TokenizationStreamResult>>, Status>;

#[cfg_attr(test, derive(Default))]
#[derive(Clone)]
pub struct ChunkerClient {
    clients: HashMap<String, ChunkersServiceClient<LoadBalancedChannel>>,
}

impl ChunkerClient {
    pub async fn new(default_port: u16, config: &[(String, ServiceConfig)]) -> Self {
        let clients = create_grpc_clients(default_port, config, ChunkersServiceClient::new).await;
        Self { clients }
    }

    fn client(&self, model_id: &str) -> Result<ChunkersServiceClient<LoadBalancedChannel>, Error> {
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
        Ok(self
            .client(model_id)?
            .tokenization_task_predict(request_with_model_id(request, model_id))
            .await?
            .into_inner())
    }

    pub async fn bidi_streaming_tokenization_task_predict(
        &self,
        model_id: &str,
        request_stream: Pin<Box<dyn Stream<Item = BidiStreamingTokenizationTaskRequest> + Send>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TokenizationStreamResult, Status>> + Send>>, Error>
    {
        let mut client = self.client(model_id)?;
        let request = request_with_model_id(request_stream, model_id);
        // NOTE: this is an ugly workaround to avoid bogus higher-ranked lifetime errors.
        // https://github.com/rust-lang/rust/issues/110338
        let response_stream_fut: Pin<Box<dyn Future<Output = StreamingTokenizationResult> + Send>> =
            Box::pin(client.bidi_streaming_tokenization_task_predict(request));
        Ok(response_stream_fut.await?.into_inner().boxed())
    }
}

fn request_with_model_id<T>(request: T, model_id: &str) -> Request<T> {
    let mut request = Request::new(request);
    request
        .metadata_mut()
        .insert(MODEL_ID_HEADER_NAME, model_id.parse().unwrap());
    request
}
