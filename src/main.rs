use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use clap::Parser;
use fms_orchestr8::{server, detector_map_config::DetectorMap};

/// App Configuration
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(default_value = "8033", long, short, env)]
    rest_port: u16,
    #[clap(long, env)]
    json_output: bool,
    #[clap(default_value="config/configmap.yaml", long, env = "DETECTOR_MAP_CONFIG")]
    detector_map_config: String,
    #[clap(long, env = "TLS_CERT_PATH")]
    tls_cert_path: Option<String>,
    #[clap(long, env = "TLS_KEY_PATH")]
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

    // Load detector map config
    let detector_map = DetectorMap::load(args.detector_map_config);

    println!("{:?}", detector_map);

    // Launch Tokio runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let rest_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.rest_port);

            server::run(
                rest_addr,
                // args.tls_cert_path
                //     .map(|cp| (cp, args.tls_key_path.unwrap())),
                // detector_map,
            )
            .await;

            Ok(())
        })
}