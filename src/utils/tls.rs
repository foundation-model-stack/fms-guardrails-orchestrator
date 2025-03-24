use std::{fs::File, io, path::PathBuf, sync::Arc};

use http_serde::http::StatusCode;
use hyper_rustls::ConfigBuilderExt;
use rustls::{
    ClientConfig, DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
};
use serde::Deserialize;

use crate::{clients, config::TlsConfig};

/// Client TLS configuration errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read TLS certs from file: {0}")]
    FailedReadCerts(io::Error),
    #[error("failed to read TLS private key from file: {0}")]
    FailedReadKey(io::Error),
    #[error("failed to read TLS CA certs from file: {0}")]
    FailedReadCaCerts(io::Error),
    #[error("missing TLS private key")]
    MissingTlsKey,
    #[error("TLS configuration error: {0}")]
    RustlsError(#[from] rustls::Error),
}

impl Error {
    /// Transforms TLS errors into internal client errors.
    pub fn into_client_error(self) -> clients::Error {
        clients::Error::Http {
            code: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("client TLS configuration failed: {}", self),
        }
    }
}

/// A no verification `rustls::verify::ServerCertVerifier` for insecure TLS configurations
/// Functionally identical to using `reqwest::ClientBuilder::danger_accept_invalid_certs(true)`.
#[derive(Debug)]
struct NoVerifier;

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer,
        _intermediates: &[CertificateDer],
        _server_name: &ServerName,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

/// Client TLS configuration builder.
#[cfg_attr(test, derive(Default))]
#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfigBuilder {
    pub cert_path: PathBuf,
    pub key_path: Option<PathBuf>,
    pub ca_cert_path: Option<PathBuf>,
    pub insecure: Option<bool>,
}

/// A resolved TLS config, built by the `TlsConfigBuilder`.
#[derive(Debug)]
pub struct ResolvedTlsConfig<'a> {
    pub cert: Vec<CertificateDer<'a>>,
    pub key: Option<PrivateKeyDer<'a>>,
    pub ca_cert: Option<Vec<CertificateDer<'a>>>,
    pub insecure: bool,
}

impl Clone for ResolvedTlsConfig<'_> {
    fn clone(&self) -> Self {
        let key = self.key.as_ref().map(|key| key.clone_key());
        Self {
            cert: self.cert.clone(),
            key,
            ca_cert: self.ca_cert.clone(),
            insecure: self.insecure,
        }
    }
}

impl TlsConfigBuilder {
    pub fn from_parts(
        cert_path: PathBuf,
        key_path: Option<PathBuf>,
        ca_cert_path: Option<PathBuf>,
        insecure: Option<bool>,
    ) -> Self {
        Self {
            cert_path,
            key_path,
            ca_cert_path,
            insecure,
        }
    }

    pub async fn build<'a>(self) -> Result<ResolvedTlsConfig<'a>, Error> {
        use Error::*;

        // Certs
        let file = File::open(self.cert_path).map_err(FailedReadCerts)?;
        let mut buf = io::BufReader::new(file);
        let cert = rustls_pemfile::certs(&mut buf)
            .collect::<Result<Vec<_>, io::Error>>()
            .map_err(FailedReadCerts)?;

        // Private key
        let key = match self.key_path {
            Some(path) => {
                let file = File::open(path).map_err(FailedReadKey)?;
                let mut buf = io::BufReader::new(file);
                Some(
                    rustls_pemfile::private_key(&mut buf)
                        .map_err(FailedReadKey)?
                        .ok_or(MissingTlsKey)?,
                )
            }
            None => None,
        };

        // CA certs
        let ca_cert = match self.ca_cert_path {
            Some(path) => {
                let file = File::open(path).map_err(FailedReadCaCerts)?;
                let mut buf = io::BufReader::new(file);
                Some(
                    rustls_pemfile::certs(&mut buf)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(FailedReadCaCerts)?,
                )
            }
            None => None,
        };

        // Insecure
        let insecure = self.insecure.unwrap_or(false);

        Ok(ResolvedTlsConfig {
            cert,
            key,
            ca_cert,
            insecure,
        })
    }
}

/// Builds and insecure TLS client config when no `TlsConfig` is provided (assumes no client auth).
pub fn build_insecure_client_config() -> ClientConfig {
    let mut config = ClientConfig::builder()
        .with_native_roots()
        .unwrap_or(ClientConfig::builder().with_webpki_roots())
        .with_no_client_auth();
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(NoVerifier));
    config
}

/// Builds a TLS client config based on the provided `TlsConfig`.
pub async fn build_client_config(tls_config: &TlsConfig) -> Result<ClientConfig, Error> {
    // Resolve the TLS config
    let tls_config = TlsConfigBuilder::from_parts(
        tls_config.cert_path.clone().unwrap(),
        tls_config.key_path.clone(),
        tls_config.client_ca_cert_path.clone(),
        tls_config.insecure,
    )
    .build()
    .await?;

    // Add CA certs, if any
    let client_config_builder = match &tls_config.ca_cert {
        Some(ca_cert) if !ca_cert.is_empty() => {
            let mut root = rustls::RootCertStore::empty();
            ca_cert.clone().iter().for_each(|certs| {
                let (_, _) = root.add_parsable_certificates([certs.to_owned()]);
            });
            ClientConfig::builder().with_root_certificates(root)
        }
        _ => ClientConfig::builder()
            .with_native_roots()
            .unwrap_or(ClientConfig::builder().with_webpki_roots()),
    };

    // Add certs and private key, if any
    let mut client_config = match &tls_config.key {
        Some(key) if !&tls_config.cert.is_empty() => {
            client_config_builder.with_client_auth_cert(tls_config.cert.clone(), key.clone_key())?
        }
        _ => client_config_builder.with_no_client_auth(),
    };

    // Remove verification if insecure
    if tls_config.insecure {
        client_config
            .dangerous()
            .set_certificate_verifier(Arc::new(NoVerifier));
    };

    Ok(client_config)
}
