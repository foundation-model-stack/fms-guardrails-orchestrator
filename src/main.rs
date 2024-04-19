use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use clap::Parser;
use fms_orchestr8::{config::OrchestratorConfig, orchestrator, server};

/// App Configuration
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(default_value = "8033", long, short, env)]
    rest_port: u16,
    #[clap(long, env)]
    json_output: bool,
    #[clap(default_value="config/config.yaml", long, env)]
    orchestrator_config: String,
    #[clap(long, env)]
    tls_cert_path: Option<String>,
    #[clap(long, env)]
    tls_key_path: Option<String>
    // TODO: Add TLS configuration for other servers or get them via above detector config
    // TODO: Add router hostname, port and TLS config
    // TODO: Add chunker hostname, port and TLS config (for now we will assume that all chunkers live locally)
}

fn main() -> Result<(), std::io::Error> {
    //Get args
    let args = Args::parse();

    // if args.tls_key_path.is_some() != args.tls_cert_path.is_some() {
    //     panic!("tls: must provide both cert and key")
    // }

    // Load orchestrator config - load could move to orchestrator if necessary
    let orchestrator_config = OrchestratorConfig::load(args.orchestrator_config);
    println!("{:?}", orchestrator_config);

    let chunker_map = orchestrator::preprocess_detector_map(orchestrator_config.clone().detector_config);
    match chunker_map {
        Ok(v) => println!("hashmap: {v:?}"),
        Err(e) => println!("error: {e:?}"),
    }

    // Launch Tokio runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let rest_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.rest_port);

            server::run(
                rest_addr,
                orchestrator_config,
            )
            .await;

            Ok(())
        })
}