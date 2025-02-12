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

use std::sync::Arc;

use fms_guardrails_orchestr8::{
    config::OrchestratorConfig, orchestrator::Orchestrator, server::ServerState,
};
use mocktail::server::HttpMockServer;
use rustls::crypto::ring;

use super::{chunker::MockChunkersServiceServer, generation::MockNlpServiceServer};

/// Default orchestrator configuration file for integration tests.
pub const CONFIG_FILE_PATH: &str = "tests/test.config.yaml";

pub fn ensure_global_rustls_state() {
    let _ = ring::default_provider().install_default();
}

/// Starts mock servers and adds them to orchestrator configuration.
pub async fn create_orchestrator_shared_state(
    generation_server: Option<&MockNlpServiceServer>,
    detectors: Vec<HttpMockServer>,
    chunkers: Vec<(&str, MockChunkersServiceServer)>,
) -> Result<Arc<ServerState>, mocktail::Error> {
    let mut config = OrchestratorConfig::load(CONFIG_FILE_PATH).await.unwrap();

    if let Some(generation_server) = generation_server {
        generation_server.start().await?;
        config.generation.as_mut().unwrap().service.port = Some(generation_server.addr().port());
    }

    for detector_mock_server in detectors {
        detector_mock_server.start().await?;

        // assign mock server port to detector config
        config
            .detectors
            .get_mut(detector_mock_server.name())
            .unwrap()
            .service
            .port = Some(detector_mock_server.addr().port());
    }

    for (chunker_name, chunker_mock_server) in chunkers {
        chunker_mock_server.start().await?;

        // assign mock server port to chunker config
        config
            .chunkers
            .as_mut()
            .unwrap()
            .get_mut(chunker_name)
            .unwrap()
            .service
            .port = Some(chunker_mock_server.addr().port());
    }

    let orchestrator = Orchestrator::new(config, false).await.unwrap();
    Ok(Arc::new(ServerState::new(orchestrator)))
}
