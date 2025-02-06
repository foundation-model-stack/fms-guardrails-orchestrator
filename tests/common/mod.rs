use rustls::crypto::ring;

pub const CONFIG_FILE_PATH: &str = "tests/test.config.yaml";

pub fn ensure_global_rustls_state() {
    let _ = ring::default_provider().install_default();
}
