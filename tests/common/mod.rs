use mocktail::generate_grpc_server;
use mocktail::mock::MockSet;
use rustls::crypto::ring;

generate_grpc_server!(
    "caikit.runtime.Chunkers.ChunkersService",
    MockChunkersServiceServer
);

pub const CONFIG_FILE_PATH: &str = "tests/test.config.yaml";

pub fn ensure_global_rustls_state() {
    let _ = ring::default_provider().install_default();
}
