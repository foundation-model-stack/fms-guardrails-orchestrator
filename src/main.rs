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

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    panic,
    path::PathBuf,
};

use clap::Parser;
use fms_guardrails_orchestr8::server;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(default_value = "8033", long, env)]
    http_port: u16,
    #[clap(default_value = "8034", long, env)]
    health_http_port: u16,
    #[clap(long, env)]
    json_output: bool,
    #[clap(default_value = "config/config.yaml", long, env)]
    config_path: PathBuf,
    #[clap(long, env)]
    tls_cert_path: Option<PathBuf>,
    #[clap(long, env)]
    tls_key_path: Option<PathBuf>,
    #[clap(long, env)]
    tls_client_ca_cert_path: Option<PathBuf>,
}

/// Panics should catch bugs in the code not errors caused by users.
/// We can make this distinction between user errors and system bugs clearer with a custom panic hook.
/// Panics should not be used for fatal user errors, use fatal! to report error and exit with a
/// user-facing error code and message instead of a trace dumped by a panic.
fn setup_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .unwrap_or(&"something went wrong rendering panic message");

        // Print the location of the panic, if available
        if let Some(location) = info.location() {
            eprintln!(
                "internal_unhandled_panic[{}:{}] {}",
                location.file(),
                location.line(),
                payload
            );
        } else {
            eprintln!("internal_unhandled_panic[?] {}", payload);
        }
    }));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_panic_hook();

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

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or(EnvFilter::new("INFO"))
        .add_directive("ginepro=info".parse().unwrap());
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

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
            server::run(
                http_addr,
                health_http_addr,
                args.tls_cert_path,
                args.tls_key_path,
                args.tls_client_ca_cert_path,
                args.config_path,
            )
            .await?;
            Ok(())
        })
}
