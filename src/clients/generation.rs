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
use hyper::{HeaderMap, StatusCode};
use tracing::warn;

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

async fn retry_function<F, Fut, Res>(max_retries: usize, func: F) -> Result<Res, Error>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<Res, Error>>,
{
    let mut attempt = 0;

    let allowed_retry_codes = [
        StatusCode::BAD_GATEWAY,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::GATEWAY_TIMEOUT,
        StatusCode::HTTP_VERSION_NOT_SUPPORTED,
        StatusCode::VARIANT_ALSO_NEGOTIATES,
    ];
    loop {
        attempt += 1;

        match func().await {
            Ok(res) => return Ok(res),

            Err(error) => {
                if allowed_retry_codes.contains(&error.status_code()) {
                    // Only retry when status code is within list
                    if attempt > max_retries {
                        warn!(
                            "Final attempt failed to connect to server. attempt: {}, error: {}",
                            attempt, &error
                        );
                        return Err(error);
                    }
                    // Exponential backoff for retries.
                    tokio::time::sleep(std::time::Duration::from_millis(
                        2_u64.pow(attempt as u32 - 1),
                    ))
                    .await;
                    warn!(
                        "failed to connect to server. attempt: {}, error: {}",
                        attempt, &error
                    );
                    continue;
                }

                // Else return error
                return Err(error);
            }
        }
    }
}

#[derive(Clone)]
pub struct GenerationClient(Option<GenerationClientInner>, usize);

#[derive(Clone)]
enum GenerationClientInner {
    Tgis(TgisClient),
    Nlp(NlpClient),
}

impl GenerationClient {
    pub fn tgis(client: TgisClient, max_retries: usize) -> Self {
        Self(Some(GenerationClientInner::Tgis(client)), max_retries)
    }

    pub fn nlp(client: NlpClient, max_retries: usize) -> Self {
        Self(Some(GenerationClientInner::Nlp(client)), max_retries)
    }

    pub fn not_configured() -> Self {
        Self(None, 0)
    }

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
                let mut response = client.tokenize(request, headers).await?;
                let response = response.responses.swap_remove(0);
                Ok((response.token_count, response.tokens))
            }
            Some(GenerationClientInner::Nlp(client)) => {
                let request = TokenizationTaskRequest { text };
                let response = retry_function(self.1, || {
                    client.tokenization_task_predict(&model_id, request.clone(), headers.clone())
                })
                .await?;
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
                let response = client.generate(request, headers).await?;
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
                let response = retry_function(self.1, || {
                    client.text_generation_task_predict(&model_id, request.clone(), headers.clone())
                })
                .await?;
                Ok(response.into())
            }
            None => Err(Error::ModelNotFound { model_id }),
        }
    }

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

                let response_stream = retry_function(self.1, || {
                    client.server_streaming_text_generation_task_predict(
                        &model_id,
                        request.clone(),
                        headers.clone(),
                    )
                })
                .await?
                .map_ok(Into::into)
                .boxed();

                Ok(response_stream)
            }
            None => Err(Error::ModelNotFound { model_id }),
        }
    }
}

#[async_trait]
impl Client for GenerationClient {
    fn name(&self) -> &str {
        "generation"
    }

    async fn health(&self) -> HealthCheckResult {
        match &self.0 {
            Some(GenerationClientInner::Tgis(client)) => client.health().await,
            Some(GenerationClientInner::Nlp(client)) => client.health().await,
            None => unimplemented!(),
        }
    }
}
