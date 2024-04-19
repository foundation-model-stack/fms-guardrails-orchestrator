

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

use crate::{models, ErrorResponse, GuardrailsResponse};
use crate::{clients::tgis::{self, GenerationServicer}};
use crate::{config::{ServiceAddr, OrchestratorConfig}};
use crate::{pb::fmaas::{
    generation_service_server::GenerationService,
    GenerationRequest, GenerationResponse,  Parameters,
    SingleGenerationRequest,
}};



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


// =========================================== Util functions ==============================================


async fn load_pem(path: String, name: &str) -> Vec<u8> {
    read(&path)
        .await
        .unwrap_or_else(|_| panic!("couldn't load {name} from {path}"))
}
