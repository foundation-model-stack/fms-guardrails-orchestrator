// Adapted from https://github.com/IBM/text-generation-router
// This is intended for use for the chunking/tokenization GRPC API until
// the REST API is able to support client streaming.
use std::collections::HashMap;

use ginepro::LoadBalancedChannel;
use tonic::{transport::ClientTlsConfig, Code, Request, Response, Status, Streaming};
use tracing::{debug, instrument};
use futures::stream::iter;

use crate::{pb::{
    caikit::runtime::nlp::{
        nlp_service_client::NlpServiceClient, nlp_service_server::NlpService,
        BidiStreamingTokenizationTaskRequest,
        ServerStreamingTextGenerationTaskRequest,
        TextGenerationTaskRequest,
        TokenizationTaskRequest,
    },
    caikit_data_model::{
        nlp::{
            GeneratedTextResult, GeneratedTextStreamResult,
            TokenizationResults, TokenizationStreamResult,
        },
    },
}, create_clients, config::ServiceAddr};

pub const METADATA_NAME_MODEL_ID: &str = "mm-model-id";

#[derive(Debug, Default, Clone)]
pub struct NlpServicer {
    clients: HashMap<String, NlpServiceClient<LoadBalancedChannel>>,
}

impl NlpServicer {
    pub async fn new(
        default_target_port: u16,
        client_tls: Option<&ClientTlsConfig>,
        model_map: &HashMap<String, ServiceAddr>,
    ) -> Self {
        let clients = create_clients(
            default_target_port, client_tls, model_map, NlpServiceClient::new
        ).await;
        Self { clients }
    }

    async fn client(
        &self,
        model_id: &str,
    ) -> Result<NlpServiceClient<LoadBalancedChannel>, Status> {
        // TODO: Fix below model mapping
        Ok(self
            .clients
            .get("gen-all-models")
            .ok_or_else(|| Status::not_found(format!("Unrecognized model_id: {model_id}")))?
            .clone())
    }
}

#[tonic::async_trait]
impl NlpService for NlpServicer {

    #[instrument(skip_all)]
    async fn tokenization_task_predict(
        &self,
        request: Request<TokenizationTaskRequest>,
    ) -> Result<Response<TokenizationResults>, Status> {
        let model_id = extract_model_id(&request)?;
        let br = request.get_ref();
        // TODO: Verify if this makes sense
        if br.text.is_empty() {
            return Ok(Response::new(TokenizationResults::default()));
        }
        debug!(
            "Performing tokenization task predict request for Model ID {}",
            model_id
        );
        self.client(model_id)
            .await?
            .tokenization_task_predict(request)
            .await
    }

    type BidiStreamingTokenizationTaskPredictStream =
        Streaming<TokenizationStreamResult>;
    #[instrument(skip_all)]
    async fn bidi_streaming_tokenization_task_predict(
        &self,
        request: Request<Streaming<BidiStreamingTokenizationTaskRequest>>,
    ) -> Result<Response<Self::BidiStreamingTokenizationTaskPredictStream>, Status> {
        let model_id = extract_model_id(&request)?;
        let br = request.get_ref();
        // TODO: Empty case should look different for streaming
        debug!(
            "Performing bidirectional streaming tokenization task predict request for Model ID {}",
            model_id
        );

        // TODO: fake request here - need to update request above to be
        // expected type constructed from TGIS response, appears to be server type?
        let stream = tonic::Request::new(iter(vec![
            BidiStreamingTokenizationTaskRequest {text_stream: String::from("moo") },
            BidiStreamingTokenizationTaskRequest {text_stream: String::from("moo") },
        ]));
        self.client(model_id)
            .await?
            .bidi_streaming_tokenization_task_predict(stream)
            .await
    }

    type ServerStreamingTextGenerationTaskPredictStream = Streaming<GeneratedTextStreamResult>;
    #[instrument(skip_all)]
    async fn server_streaming_text_generation_task_predict(
        &self,
        request: Request<ServerStreamingTextGenerationTaskRequest>,
    ) -> Result<Response<Self::ServerStreamingTextGenerationTaskPredictStream>, Status> {
        // let sstr = request.get_ref();
        let model_id = extract_model_id(&request)?;
        debug!(
            "Routing text generation streaming generation request for Model ID {}",
            model_id
        );
        self.client(model_id)
            .await?
            .server_streaming_text_generation_task_predict(request)
            .await

    }

    #[instrument(skip_all)]
    async fn text_generation_task_predict(
        &self,
        _request: Request<TextGenerationTaskRequest>,
    ) -> Result<Response<GeneratedTextResult>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

}

/// Extracts model_id from [`Request`] metadata.
fn extract_model_id<T>(request: &Request<T>) -> Result<&str, Status> {
    let metadata = request.metadata();
    if !metadata.contains_key(METADATA_NAME_MODEL_ID) {
        return Err(Status::new(
            Code::InvalidArgument,
            "Missing required model ID",
        ));
    }
    let model_id = metadata
        .get(METADATA_NAME_MODEL_ID)
        .unwrap()
        .to_str()
        .unwrap();
    Ok(model_id)
}