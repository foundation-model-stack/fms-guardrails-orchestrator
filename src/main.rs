use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
