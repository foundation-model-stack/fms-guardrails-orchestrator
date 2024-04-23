

use std::{collections::HashMap, hash::Hash};
use std::convert::Infallible;

use axum::{
    extract::{Extension, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, sse::{Event, KeepAlive, Sse}},
    Json,
};
use futures::{stream::Stream, StreamExt};
use tonic::transport::{
    server::RoutesBuilder, Certificate, ClientTlsConfig, Identity, Server, ServerTlsConfig,
};
use tokio::fs::read;
use tracing::{error, info};

use crate::{models, ErrorResponse, GuardrailsResponse};
use crate::{
    clients::tgis::{self, GenerationServicer},
    clients::nlp::{NlpServicer, METADATA_NAME_MODEL_ID}
};
use crate::{config::{ServiceAddr, OrchestratorConfig}};
use crate::{pb::fmaas::{
    generation_service_server::GenerationService,
    GenerationRequest, GenerationResponse,  Parameters,
    SingleGenerationRequest,
}};
use crate::{pb::caikit::runtime::nlp::{
    nlp_service_server::NlpService,
    ServerStreamingTextGenerationTaskRequest,
    TokenClassificationTaskRequest
}};
use crate::pb::caikit_data_model::nlp::TokenClassificationResults;




// =========================================== Client Calls ==============================================

pub async fn configure_tgis(
    service_addr: ServiceAddr,
    default_target_port: u16,
) -> GenerationServicer {

    // NOTE: We only want to configure and connect to 1 TGIS "router"
    let model_map = HashMap::from([("tgis-all-models".to_owned(), service_addr.clone())]);

    // Configure TLS if requested
    let mut client_tls = service_addr.tls_enabled.then_some(ClientTlsConfig::new());
    if let Some(cert_path) = service_addr.tls_ca_path {
        // info!("Configuring TLS for outgoing connections to model servers");
        let cert_pem = load_pem(cert_path, "cert").await;
        let cert = Certificate::from_pem(cert_pem);
        client_tls = client_tls.map(|c| c.ca_certificate(cert));
    }
    let generation_servicer =
            GenerationServicer::new(default_target_port, client_tls.as_ref(), &model_map);
    generation_servicer.await
}

pub async fn call_tgis_stream(
    Json(payload): Json<models::GuardrailsHttpRequest>,
    tgis_servicer: GenerationServicer,
    on_message_callback: impl Fn(models::ClassifiedGeneratedTextStreamResult) -> Event,
)  -> impl Stream<Item = Result<Event, Infallible>> {
    // TODO: Add remaining parameter
    let mut tgis_request = tonic::Request::new(
        SingleGenerationRequest {
            model_id: payload.model_id,
            request: Some(GenerationRequest {text: payload.inputs}),
            prefix_id: None,
            params: None,

            // prefix_id: Some("".to_string()),
            // params: None,
        }
    );

    let mut index: i32 = 0;
    // TODO: Fix hardcoded start index
    let start_index: i32 = 0;
    let stream = async_stream::stream! {
        // Server sending event stream
        // TODO: Currently following is considering successfully response. We need to put it under match to handle potential errors.
        let mut result = tgis_servicer.generate_stream(tgis_request).await.unwrap().into_inner();

        while let Some(item) = result.next().await  {
            match item {
                Ok(gen_response) => {
                    let tgis_r = gen_response;
                    println!("{:?}", tgis_r);
                    let mut stream_token = models::ClassifiedGeneratedTextStreamResult::new(
                        // TODO: Implement real text gen token classification results
                        models::TextGenTokenClassificationResults::new(),
                        tgis_r.input_token_count as i32,
                        start_index
                    );
                    stream_token.generated_text = Some(tgis_r.text);
                    stream_token.processed_index = index.into();
                    index += 1;
                    let event = on_message_callback(stream_token);
                    yield Ok(event);
                }
                status => print!("{:?}", status)
            }

        }
    };
    stream
}



pub async fn configure_nlp(
    service_addr: ServiceAddr,
    default_target_port: u16,
) -> NlpServicer {

    // NOTE: We only want to configure and connect to 1 caikit nlp service which will send request to all
    let model_map = HashMap::from([("gen-all-models".to_owned(), service_addr.clone())]);

    // Configure TLS if requested
    let mut client_tls = service_addr.tls_enabled.then_some(ClientTlsConfig::new());
    if let Some(cert_path) = service_addr.tls_ca_path {
        // info!("Configuring TLS for outgoing connections to model servers");
        let cert_pem = load_pem(cert_path, "cert").await;
        let cert = Certificate::from_pem(cert_pem);
        client_tls = client_tls.map(|c| c.ca_certificate(cert));
    }
    let nlp_servicer =
        NlpServicer::new(default_target_port, client_tls.as_ref(), &model_map);
    nlp_servicer.await
}

pub async fn call_nlp_text_gen_stream (
    Json(payload): Json<models::GuardrailsHttpRequest>,
    nlp_servicer: NlpServicer,
    on_message_callback: impl Fn(models::ClassifiedGeneratedTextStreamResult) -> Event,
) -> impl Stream<Item = Result<Event, Infallible>> {

    // TODO: Add remaining parameter
    let mut nlp_request = tonic::Request::new(
        ServerStreamingTextGenerationTaskRequest::new(payload.inputs)
    );

    nlp_request.metadata_mut().insert(METADATA_NAME_MODEL_ID, payload.model_id.parse().unwrap());

    let mut index: i32 = 0;

    // TODO: Fix hardcoded start index
    let start_index: i32 = 0;
    // TODO: Implement proper error handling for cases when we receive error connecting to server
    // or when server returns error
    let stream = async_stream::stream! {
        // Server sending event stream
        let result = nlp_servicer.server_streaming_text_generation_task_predict(nlp_request).await;

        match result {
            Ok(response) => {
                let mut result = response.into_inner();
                while let Some(item) = result.next().await  {
                    match item {
                        Ok(gen_response) => {
                            let nlp_r = gen_response;
                            println!("{:?}", nlp_r);
                            let mut stream_token = models::ClassifiedGeneratedTextStreamResult::new(
                                // TODO: Implement real text gen token classification results
                                models::TextGenTokenClassificationResults::new(),
                                nlp_r.details.unwrap().input_token_count as i32,
                                start_index
                            );
                            stream_token.generated_text = Some(nlp_r.generated_text);
                            stream_token.processed_index = index.into();
                            index += 1;
                            let event = on_message_callback(stream_token);
                            yield Ok(event);
                        }
                        status => println!("{:?}", status)
                    }
                }
            }
            Err(error) => {
                error!("error response from caikit-nlp: {:?}", error);

                let err = Err(ErrorResponse{error: "error response from caikit-nlp".to_string()});
                yield Ok(Event::from(err.expect("Error handling not implemented")));
            //    panic!("{}", error.message().to_string())
            }//.unwrap() //.expect("error response from caikit-nlp")
        }

    };
    stream
}


pub async fn call_nlp_token_classification (
    text: String,
    model_id: String,
    params: Option<HashMap<String, Box<dyn std::any::Any>>>,
    nlp_servicer: NlpServicer,
) -> Result<TokenClassificationResults, ErrorResponse> {

    // TODO: Get real parameters from params and splat them into the section below
    let mut nlp_request = tonic::Request::new(
        TokenClassificationTaskRequest {
            text: text,
            threshold: None
        }
    );

    nlp_request.metadata_mut().insert(METADATA_NAME_MODEL_ID, model_id.parse().unwrap());

    let result = nlp_servicer.token_classification_task_predict(nlp_request).await;

    match result {
        Ok(response) => {
            return Ok(response.get_ref().to_owned());
        }
        Err(error) => {
            error!("error response from caikit-nlp: {:?}", error);
            Err(ErrorResponse{error: error.message().to_string()})
        }
    }
}



// pub async fn detector_streamify (
//     text: String,
//     model_id: String,
//     params: Option<HashMap<String, Box<dyn std::any::Any>>>,
//     nlp_servicer: NlpServicer,
// ) -> impl Stream<Item = Result<Event, Infallible>> {

//     let token_class_result =
//         call_nlp_token_classification(
//             text,
//             model_id,
//             params,
//             nlp_servicer).await;

//     let response_stream = sync_stream::stream! {
//         token
//     };
//     // How to convert non stream to stream.
//     // let token_class_to_stream = Stream<Item = Result<TokenClassificationResults, ErrorResponse>>::new( {
//     //     stream::unfold((), |()| async { Some((token_class_result.await, ())) })
//     // });
//     // let token_class_strm = stream::once(token_class_result);

//     // let response_stream = match token_class_result.await {
//     //     // TODO: Add logic to parse and handle token_class_result properly
//     //     Ok(value) => {
//     //         print!("Response from token class result: {:?}", value);

//     //     // utils::call_tgis_stream(Json(payload), state.tgis_servicer.clone(), on_message_callback).await;
//     //     }
//     //     error => {
//     //         println!("{:?}", error);
//     //         // let error_res = stream::once(async {Err(ErrorResponse {error: "token classification failed".to_string()});
//     //         // let error_strm = stream::repeat_with(|| Event::default().data(Err(ErrorResponse {error: "token classification failed".to_string()}))).map(Err);

//     //         let error_strm = async_stream::stream! {
//     //             Err(ErrorResponse {error: "token classification failed".to_string()})
//     //         };

//     //     }
//     // };
//     response_stream
// }


// =========================================== Util functions ==============================================


async fn load_pem(path: String, name: &str) -> Vec<u8> {
    read(&path)
        .await
        .unwrap_or_else(|_| panic!("couldn't load {name} from {path}"))
}

// Add initialization method to the request
impl ServerStreamingTextGenerationTaskRequest {
    #[allow(clippy::new_without_default)]
    pub fn new(text: String) -> ServerStreamingTextGenerationTaskRequest {
        ServerStreamingTextGenerationTaskRequest {
            text,
            max_new_tokens: None,
            min_new_tokens: None,
            truncate_input_tokens: None,
            decoding_method: None,
            top_k: None,
            top_p: None,
            typical_p: None,
            temperature: None,
            repetition_penalty: None,
            max_time: None,
            exponential_decay_length_penalty: None,
            stop_sequences: [].to_vec(),
            seed: None,
            preserve_input_text: None,
            input_tokens: None,
            generated_tokens: None,
            token_logprobs: None,
            token_ranks: None,
        }
    }
}