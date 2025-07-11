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

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use clap::Parser;
use fms_guardrails_orchestr8::{
    args::Args, config::OrchestratorConfig, orchestrator::Orchestrator, server, utils,
};
use tracing::info;

fn main() -> Result<(), anyhow::Error> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let args = Args::parse();
    if args.tls_key_path.is_some() != args.tls_cert_path.is_some() {
        panic!("tls: must provide both cert and key")
    }
    if args.tls_client_ca_cert_path.is_some() && args.tls_cert_path.is_none() {
        panic!("tls: cannot provide client ca cert without keypair")
    }

    let http_addr: SocketAddr =
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.http_port);
    let health_http_addr: SocketAddr =
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.health_http_port);

    // Launch Tokio runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let trace_shutdown = utils::trace::init_tracing(args.clone().into())?;
            let config = OrchestratorConfig::load(args.config_path).await?;
            let orchestrator = Orchestrator::new(config, args.start_up_health_check).await?;

            let (health_handle, guardrails_handle) = server::run(
                http_addr,
                health_http_addr,
                args.tls_cert_path,
                args.tls_key_path,
                args.tls_client_ca_cert_path,
                orchestrator,
            )
            .await
            .unwrap_or_else(|e| panic!("failed to run server: {e}"));

            // Await server shutdown
            let _ = tokio::join!(health_handle, guardrails_handle);
            info!("shutdown complete");

            trace_shutdown()
        })
}
