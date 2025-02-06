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

use fms_guardrails_orchestr8::config::OrchestratorConfig;
use fms_guardrails_orchestr8::orchestrator::Orchestrator;
use fms_guardrails_orchestr8::server::ServerState;
use mocktail::generate_grpc_server;
use mocktail::mock::MockSet;
use mocktail::server::HttpMockServer;
use rustls::crypto::ring;

generate_grpc_server!(
    "caikit.runtime.Chunkers.ChunkersService",
    MockChunkersServiceServer
);

/// Default orchestrator configuration file for integration tests.
pub const CONFIG_FILE_PATH: &str = "tests/test.config.yaml";

///
pub fn ensure_global_rustls_state() {
    let _ = ring::default_provider().install_default();
}

/// Creates an orchestrator shared state based off from the default test configuration file and given detector mocks.
pub async fn create_orchestrator_shared_state(detectors: Vec<HttpMockServer>) -> Arc<ServerState> {
    let mut config = OrchestratorConfig::load(CONFIG_FILE_PATH).await.unwrap();

    for detector_mock_server in detectors {
        let _ = detector_mock_server.start().await;

        // assign mock server port to detector config
        config
            .detectors
            .get_mut(detector_mock_server.name())
            .unwrap()
            .service
            .port = Some(detector_mock_server.addr().port());
    }

    let orchestrator = Orchestrator::new(config, false).await.unwrap();
    Arc::new(ServerState::new(orchestrator))
}
