use futures::StreamExt;
use tokio::sync::mpsc;
use tracing::debug;

use super::{Error, NlpClient, TgisClient};
use crate::{
    models::{
        ClassifiedGeneratedTextResult, ClassifiedGeneratedTextStreamResult,
        GuardrailsTextGenerationParameters,
    },
    pb::{
        caikit::runtime::nlp::{
            ServerStreamingTextGenerationTaskRequest, TextGenerationTaskRequest,
            TokenizationTaskRequest,
        },
        fmaas::{
            BatchedGenerationRequest, BatchedTokenizeRequest, GenerationRequest,
            SingleGenerationRequest, TokenizeRequest,
        },
    },
};

#[derive(Clone)]
pub enum GenerationClient {
    Tgis(TgisClient),
    Nlp(NlpClient),
}

impl GenerationClient {
    pub async fn tokenize(
        &self,
        model_id: String,
        text: String,
    ) -> Result<(u32, Vec<String>), Error> {
        match self {
            GenerationClient::Tgis(client) => {
                let request = BatchedTokenizeRequest {
                    model_id: model_id.clone(),
                    requests: vec![TokenizeRequest { text }],
                    return_tokens: false,
                    return_offsets: false,
                    truncate_input_tokens: 0,
                };
                debug!(%model_id, provider = "tgis", ?request, "sending tokenize request");
                let mut response = client.tokenize(request).await?;
                debug!(%model_id, provider = "tgis", ?response, "received tokenize response");
                let response = response.responses.swap_remove(0);
                Ok((response.token_count, response.tokens))
            }
            GenerationClient::Nlp(client) => {
                let request = TokenizationTaskRequest { text };
                debug!(%model_id, provider = "nlp", ?request, "sending tokenize request");
                let response = client.tokenization_task_predict(&model_id, request).await?;
                debug!(%model_id, provider = "nlp", ?response, "received tokenize response");
                let tokens = response
                    .results
                    .into_iter()
                    .map(|token| token.text)
                    .collect::<Vec<_>>();
                Ok((response.token_count as u32, tokens))
            }
        }
    }

    pub async fn generate(
        &self,
        model_id: String,
        text: String,
        params: Option<GuardrailsTextGenerationParameters>,
    ) -> Result<ClassifiedGeneratedTextResult, Error> {
        match self {
            GenerationClient::Tgis(client) => {
                let params = params.map(Into::into);
                let request = BatchedGenerationRequest {
                    model_id: model_id.clone(),
                    prefix_id: None,
                    requests: vec![GenerationRequest { text }],
                    params,
                };
                debug!(%model_id, provider = "tgis", ?request, "sending generate request");
                let response = client.generate(request).await?;
                debug!(%model_id, provider = "tgis", ?response, "received generate response");
                Ok(response.into())
            }
            GenerationClient::Nlp(client) => {
                let request = if let Some(params) = params {
                    TextGenerationTaskRequest {
                        text,
                        max_new_tokens: params.max_new_tokens.map(|v| v as i64),
                        min_new_tokens: params.min_new_tokens.map(|v| v as i64),
                        truncate_input_tokens: params.truncate_input_tokens.map(|v| v as i64),
                        decoding_method: params.decoding_method,
                        top_k: params.top_k.map(|v| v as i64),
                        top_p: params.top_p,
                        typical_p: params.typical_p,
                        temperature: params.temperature,
                        repetition_penalty: params.repetition_penalty,
                        max_time: params.max_time,
                        exponential_decay_length_penalty: params
                            .exponential_decay_length_penalty
                            .map(Into::into),
                        stop_sequences: params.stop_sequences.unwrap_or_default(),
                        seed: params.seed.map(|v| v as u64),
                        preserve_input_text: params.preserve_input_text,
                        input_tokens: params.input_tokens,
                        generated_tokens: params.generated_tokens,
                        token_logprobs: params.token_logprobs,
                        token_ranks: params.token_ranks,
                    }
                } else {
                    TextGenerationTaskRequest {
                        text,
                        ..Default::default()
                    }
                };
                debug!(%model_id, provider = "nlp", ?request, "sending generate request");
                let response = client
                    .text_generation_task_predict(&model_id, request)
                    .await?;
                debug!(%model_id, provider = "nlp", ?response, "received generate response");
                Ok(response.into())
            }
        }
    }

    pub async fn generate_stream(
        &self,
        model_id: String,
        text: String,
        params: Option<GuardrailsTextGenerationParameters>,
    ) -> Result<mpsc::Receiver<ClassifiedGeneratedTextStreamResult>, Error> {
        let (tx, rx) = mpsc::channel(128);
        match self {
            GenerationClient::Tgis(client) => {
                let params = params.map(Into::into);
                let request = SingleGenerationRequest {
                    model_id: model_id.clone(),
                    prefix_id: None,
                    request: Some(GenerationRequest { text }),
                    params,
                };
                debug!(%model_id, provider = "tgis", ?request, "sending generate_stream request");
                let mut response_stream = client.generate_stream(request).await?;
                tokio::spawn(async move {
                    while let Some(response) = response_stream.next().await {
                        let _ = tx.send(response.into()).await;
                    }
                });
                Ok(rx)
            }
            GenerationClient::Nlp(client) => {
                let request = if let Some(params) = params {
                    ServerStreamingTextGenerationTaskRequest {
                        text,
                        max_new_tokens: params.max_new_tokens.map(|v| v as i64),
                        min_new_tokens: params.min_new_tokens.map(|v| v as i64),
                        truncate_input_tokens: params.truncate_input_tokens.map(|v| v as i64),
                        decoding_method: params.decoding_method,
                        top_k: params.top_k.map(|v| v as i64),
                        top_p: params.top_p,
                        typical_p: params.typical_p,
                        temperature: params.temperature,
                        repetition_penalty: params.repetition_penalty,
                        max_time: params.max_time,
                        exponential_decay_length_penalty: params
                            .exponential_decay_length_penalty
                            .map(Into::into),
                        stop_sequences: params.stop_sequences.unwrap_or_default(),
                        seed: params.seed.map(|v| v as u64),
                        preserve_input_text: params.preserve_input_text,
                        input_tokens: params.input_tokens,
                        generated_tokens: params.generated_tokens,
                        token_logprobs: params.token_logprobs,
                        token_ranks: params.token_ranks,
                    }
                } else {
                    ServerStreamingTextGenerationTaskRequest {
                        text,
                        ..Default::default()
                    }
                };
                debug!(%model_id, provider = "nlp", ?request, "sending generate_stream request");
                let mut response_stream = client
                    .server_streaming_text_generation_task_predict(&model_id, request)
                    .await?;
                tokio::spawn(async move {
                    while let Some(response) = response_stream.next().await {
                        let _ = tx.send(response.into()).await;
                    }
                });
                Ok(rx)
            }
        }
    }
}
