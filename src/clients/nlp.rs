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

#[faux::create]
#[derive(Clone)]
pub struct NlpClient {
    clients: HashMap<String, NlpServiceClient<LoadBalancedChannel>>,
}

#[faux::methods]
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
#[cfg(test)]
mod tests {

    use super::*;
    use crate::{
        pb::caikit_data_model::caikit_nlp::ExponentialDecayLengthPenalty,
        pb::caikit_data_model::nlp::GeneratedTextResult,
        pb::caikit_data_model::nlp::GeneratedToken,
    };

    #[tokio::test]
    async fn test_text_generation_task_predict() {
        // Expected OK case
        let exp_decay: ExponentialDecayLengthPenalty = ExponentialDecayLengthPenalty {
            start_index: 1,
            decay_factor: 2.0,
        };
        let exp_decay_mock: ExponentialDecayLengthPenalty = ExponentialDecayLengthPenalty {
            start_index: 1,
            decay_factor: 2.0,
        };
        let request_mock: TextGenerationTaskRequest = TextGenerationTaskRequest {
            max_new_tokens: Some(99),
            min_new_tokens: Some(1),
            truncate_input_tokens: Some(500),
            decoding_method: Some("SAMPLING".to_string()),
            top_k: Some(2),
            top_p: Some(0.9),
            typical_p: Some(0.5),
            temperature: Some(0.8),
            seed: Some(42),
            repetition_penalty: Some(2.0),
            max_time: Some(0.0),
            stop_sequences: vec!["42".to_string()],
            text: "Text".to_string(),
            token_logprobs: Some(true),
            token_ranks: Some(true),
            input_tokens: Some(true),
            generated_tokens: Some(true),
            preserve_input_text: Some(true),
            exponential_decay_length_penalty: Some(exp_decay_mock),
        };
        let request: TextGenerationTaskRequest = TextGenerationTaskRequest {
            max_new_tokens: Some(99),
            min_new_tokens: Some(1),
            truncate_input_tokens: Some(500),
            decoding_method: Some("SAMPLING".to_string()),
            top_k: Some(2),
            top_p: Some(0.9),
            typical_p: Some(0.5),
            temperature: Some(0.8),
            seed: Some(42),
            repetition_penalty: Some(2.0),
            max_time: Some(0.0),
            stop_sequences: vec!["42".to_string()],
            text: "Text".to_string(),
            token_logprobs: Some(true),
            token_ranks: Some(true),
            input_tokens: Some(true),
            generated_tokens: Some(true),
            preserve_input_text: Some(true),
            exponential_decay_length_penalty: Some(exp_decay),
        };
        let gen_token: GeneratedToken = GeneratedToken {
            text: "Text".to_string(),
            logprob: 0.4,
            rank: 1,
        };
        let input_token: GeneratedToken = GeneratedToken {
            text: "text".to_string(),
            logprob: 0.4,
            rank: 1,
        };

        let mut mock_nlp_client = NlpClient::faux();

        faux::when!(mock_nlp_client.text_generation_task_predict("bloom-560m", request_mock))
            .once()
            .then_return(Ok(GeneratedTextResult {
                generated_text: "Text text".to_string(),
                generated_tokens: 42,
                finish_reason: 2,
                input_token_count: 20,
                seed: 23,
                tokens: vec![gen_token],
                input_tokens: vec![input_token],
            }));
        assert!(mock_nlp_client
            .text_generation_task_predict("bloom-560m", request)
            .await
            .is_ok());
    }
}
