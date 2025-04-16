use std::collections::HashMap;

use hyper::Uri;
use tracing::error;
use url::Url;

use crate::{
    clients::chunker::DEFAULT_CHUNKER_ID,
    config::{DetectorConfig, DetectorType},
    models::DetectorParams,
    orchestrator::Error,
};

pub mod json;
pub mod tls;
pub mod trace;

/// Simple trait used to extend `url::Url` with functionality to transform into `hyper::Uri`.
pub trait AsUriExt {
    fn as_uri(&self) -> Uri;
}

impl AsUriExt for Url {
    fn as_uri(&self) -> Uri {
        Uri::try_from(self.to_string()).unwrap()
    }
}

/// Validates guardrails on request.
pub fn validate_guardrails(
    request_detectors: &HashMap<String, DetectorParams>,
    orchestrator_detectors: &HashMap<String, DetectorConfig>,
    allowed_detector_types: Vec<DetectorType>,
    allows_whole_doc_chunker: bool,
) -> Result<(), Error> {
    let whole_doc_chunker_id = DEFAULT_CHUNKER_ID;
    let detector_ids = request_detectors.keys();
    for detector_id in detector_ids {
        // validate detectors
        match orchestrator_detectors.get(detector_id) {
            Some(detector_config) => {
                if !allowed_detector_types.contains(&detector_config.r#type) {
                    let error = Error::Validation(format!(
                        "detector `{detector_id}` is not supported by this endpoint"
                    ));
                    error!(%error, "invalid detector on request");
                    return Err(error);
                }
                if !allows_whole_doc_chunker && detector_config.chunker_id == whole_doc_chunker_id {
                    let error = Error::Validation(format!(
                        "{}: detector is associated with whole_doc_chunker, which is not supported on this endpoint",
                        detector_id
                    ));
                    error!(%error, "detector with invalid chunker on request");
                    return Err(error);
                }
            }
            None => {
                let error = Error::DetectorNotFound(detector_id.clone());
                error!(%error, "detector not found");
                return Err(error);
            }
        }
    }
    Ok(())
}
