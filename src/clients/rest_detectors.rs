use serde::Deserialize;
use serde_json::json;
use std::env;
use reqwest::Client;

use crate::{create_rest_clients, detector_models};


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
        let clients = create_rest_clients(
            default_target_port, model_map, NlpServiceClient::new
        ).await;
        Self { clients }
    }
}

impl DetectorService for DetectorServicer {
    async fn classify(
        &self,
        request: detector_models::DetectorTaskRequestHttpRequest
    ) -> Result<detector_models::DetectorTaskResponseList, ErrorResponse> {

        let detector_req = request.get_ref();
        if detector_req.is_empty() {
            return Err(ErrorResponse {error: "Empty Request"})
        }

        let client_config = self.client(&detector_req.model_id).await?;

        client_config
            .client
            .post(client_config.url)
            .json(request)
            .send().await;

    }
}