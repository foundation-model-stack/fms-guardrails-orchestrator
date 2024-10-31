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
use futures::{StreamExt, TryStreamExt};
use hyper::HeaderMap;
use tracing::{debug, instrument};

use super::{BoxStream, Client, Error, NlpClient, TgisClient};
use crate::{
    health::HealthCheckResult,
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

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct GenerationClient(Option<GenerationClientInner>);

#[derive(Clone)]
enum GenerationClientInner {
    Tgis(TgisClient),
    Nlp(NlpClient),
}

#[cfg_attr(test, faux::methods)]
impl GenerationClient {
    pub fn tgis(client: TgisClient) -> Self {
        Self(Some(GenerationClientInner::Tgis(client)))
    }

    pub fn nlp(client: NlpClient) -> Self {
        Self(Some(GenerationClientInner::Nlp(client)))
    }

    pub fn not_configured() -> Self {
        Self(None)
    }

    #[instrument(skip_all, fields(model_id))]
    pub async fn tokenize(
        &self,
        model_id: String,
        text: String,
        headers: HeaderMap,
    ) -> Result<(u32, Vec<String>), Error> {
        match &self.0 {
            Some(GenerationClientInner::Tgis(client)) => {
                let request = BatchedTokenizeRequest {
                    model_id: model_id.clone(),
                    requests: vec![TokenizeRequest { text }],
                    return_tokens: false,
                    return_offsets: false,
                    truncate_input_tokens: 0,
                };
                debug!(provider = "tgis", ?request, "sending tokenize request");
                let mut response = client.tokenize(request, headers).await?;
                debug!(provider = "tgis", ?response, "received tokenize response");
                let response = response.responses.swap_remove(0);
                Ok((response.token_count, response.tokens))
            }
            Some(GenerationClientInner::Nlp(client)) => {
                let request = TokenizationTaskRequest { text };
                debug!(provider = "nlp", ?request, "sending tokenize request");
                let response = client
                    .tokenization_task_predict(&model_id, request, headers)
                    .await?;
                debug!(provider = "nlp", ?response, "received tokenize response");
                let tokens = response
                    .results
                    .into_iter()
                    .map(|token| token.text)
                    .collect::<Vec<_>>();
                Ok((response.token_count as u32, tokens))
            }
            None => Err(Error::ModelNotFound { model_id }),
        }
    }

    #[instrument(skip_all, fields(model_id))]
    pub async fn generate(
        &self,
        model_id: String,
        text: String,
        params: Option<GuardrailsTextGenerationParameters>,
        headers: HeaderMap,
    ) -> Result<ClassifiedGeneratedTextResult, Error> {
        match &self.0 {
            Some(GenerationClientInner::Tgis(client)) => {
                let params = params.map(Into::into);
                let request = BatchedGenerationRequest {
                    model_id: model_id.clone(),
                    prefix_id: None,
                    requests: vec![GenerationRequest { text }],
                    params,
                };
                debug!(provider = "tgis", ?request, "sending generate request");
                let response = client.generate(request, headers).await?;
                debug!(provider = "tgis", ?response, "received generate response");
                Ok(response.into())
            }
            Some(GenerationClientInner::Nlp(client)) => {
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
                        include_stop_sequence: params.include_stop_sequence,
                    }
                } else {
                    TextGenerationTaskRequest {
                        text,
                        ..Default::default()
                    }
                };
                debug!(provider = "nlp", ?request, "sending generate request");
                let response = client
                    .text_generation_task_predict(&model_id, request, headers)
                    .await?;
                debug!(provider = "nlp", ?response, "received generate response");
                Ok(response.into())
            }
            None => Err(Error::ModelNotFound { model_id }),
        }
    }

    #[instrument(skip_all, fields(model_id))]
    pub async fn generate_stream(
        &self,
        model_id: String,
        text: String,
        params: Option<GuardrailsTextGenerationParameters>,
        headers: HeaderMap,
    ) -> Result<BoxStream<Result<ClassifiedGeneratedTextStreamResult, Error>>, Error> {
        match &self.0 {
            Some(GenerationClientInner::Tgis(client)) => {
                let params = params.map(Into::into);
                let request = SingleGenerationRequest {
                    model_id: model_id.clone(),
                    prefix_id: None,
                    request: Some(GenerationRequest { text }),
                    params,
                };
                debug!(
                    provider = "tgis",
                    ?request,
                    "sending generate_stream request"
                );
                let response_stream = client
                    .generate_stream(request, headers)
                    .await?
                    .map_ok(Into::into)
                    .boxed();
                Ok(response_stream)
            }
            Some(GenerationClientInner::Nlp(client)) => {
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
                        include_stop_sequence: params.include_stop_sequence,
                    }
                } else {
                    ServerStreamingTextGenerationTaskRequest {
                        text,
                        ..Default::default()
                    }
                };
                debug!(
                    provider = "nlp",
                    ?request,
                    "sending generate_stream request"
                );
                let response_stream = client
                    .server_streaming_text_generation_task_predict(&model_id, request, headers)
                    .await?
                    .map_ok(Into::into)
                    .boxed();
                Ok(response_stream)
            }
            None => Err(Error::ModelNotFound { model_id }),
        }
    }
}

#[cfg_attr(test, faux::methods)]
#[async_trait]
impl Client for GenerationClient {
    fn name(&self) -> &str {
        "generation"
    }

    async fn health(&self) -> Result<HealthCheckResult, Error> {
        match &self.0 {
            Some(GenerationClientInner::Tgis(client)) => client.health().await,
            Some(GenerationClientInner::Nlp(client)) => client.health().await,
            None => unimplemented!(),
        }
    }
}
