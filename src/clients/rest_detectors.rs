use std::borrow::Borrow;
use std::collections::HashMap;


use crate::config::ServiceAddr;
use crate::{create_rest_clients, clients::detector_models};
use crate::{ErrorResponse, RestClientConfig};


pub const DETECTOR_ID_HEADER_NAME: &'static str = "detector-id";

// Struct containing map of clients,
// where each model name is mapped to a tuple of
// url (host) and client
#[derive(Debug, Default, Clone)]
pub struct DetectorServicer {
    clients: HashMap<String, RestClientConfig>,
}


impl DetectorServicer {
    pub async fn new(
        default_target_port: u16,
        model_map: &HashMap<String, ServiceAddr>,
    ) -> Self {
        let clients: HashMap<String, RestClientConfig> = create_rest_clients(
            default_target_port, model_map,
        ).await;
        Self { clients }
    }

    async fn client(
        &self,
        model_id: &str,
    ) -> Result<RestClientConfig, ErrorResponse> {
        // TODO: Fix below model mapping
        Ok(self
            .clients
            .get(&model_id.to_string())
            .ok_or_else(|| ErrorResponse{ error: format!("Unrecognized model_id: {model_id}")})?
            .clone())
    }

}


trait DetectorService {
    async fn classify(
        &self,
        model_id: String,
        request: detector_models::DetectorTaskRequestHttpRequest
    ) -> Result<detector_models::DetectorTaskResponseList, ErrorResponse> ;
}

impl DetectorService for DetectorServicer {

    async fn classify(
        &self,
        model_id: String,
        request: detector_models::DetectorTaskRequestHttpRequest
    ) -> Result<detector_models::DetectorTaskResponseList, ErrorResponse> {
        let detector_req = request.borrow();
        let model_id: &str = model_id.as_str().as_ref();
        let client_config = self.client(model_id).await?;

        let url = client_config.url;
        let response = client_config
            .client
            .post(url)
            .header(DETECTOR_ID_HEADER_NAME.to_string(), model_id)
            .json(detector_req)
            .send().await;
        response.unwrap().json().await.unwrap()
    }

}