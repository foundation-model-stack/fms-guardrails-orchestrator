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
use serde::{Deserialize, Serialize};

/// Errors returned by detector endpoints.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct DetectorError {
    pub code: u16,
    pub message: String,
}

/// Errors returned by orchestrator endpoints.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct OrchestratorError {
    pub code: u16,
    pub details: String,
}

impl OrchestratorError {
    /// Helper function that generates an orchestrator internal
    /// server error.
    pub fn internal() -> OrchestratorError {
        OrchestratorError {
            code: 500,
            details: "unexpected error occurred while processing request".into(),
        }
    }
    /// Helper function that generates an orchestrator non-existing detector error.
    pub fn detector_not_found(detector_name: &str) -> Self {
        Self {
            code: 404,
            details: format!("detector `{}` not found", detector_name),
        }
    }

    /// Helper function that generates an orchestrator invalid detector error.
    pub fn detector_not_supported(detector_name: &str) -> Self {
        Self {
            code: 422,
            details: format!(
                "detector `{}` is not supported by this endpoint",
                detector_name
            ),
        }
    }

    /// Helper function that generates an orchestrator required field error.
    pub fn required(field_name: &str) -> Self {
        Self {
            code: 422,
            details: format!("`{}` is required", field_name),
        }
    }

    /// Helper function that generates an orchestrator invalid chunker error.
    pub fn chunker_not_supported(detector_name: &str) -> Self {
        Self {
            code: 422,
            details: format!(
                "detector `{}` uses chunker `whole_doc_chunker`, which is not supported by this endpoint",
                detector_name
            ),
        }
    }
}
