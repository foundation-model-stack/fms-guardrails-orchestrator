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
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{clients::detector, models};

/// A detection.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    /// Start index of the detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<usize>,
    /// End index of the detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<usize>,
    /// Text corresponding to the detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// ID of the detector
    pub detector_id: Option<String>,
    /// Type of detection
    pub detection_type: String,
    /// Detection class
    pub detection: String,
    /// Confidence level of the detection class
    pub score: f64,
    /// Detection evidence
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<DetectionEvidence>,
    /// Detection metadata
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: models::Metadata,
}

/// Detection evidence.
#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectionEvidence {
    /// Evidence name
    pub name: String,
    /// Evidence value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Evidence score
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Additional evidence
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

/// Additional detection evidence.
#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

// Conversions

impl From<detector::ContentAnalysisResponse> for Detection {
    fn from(value: detector::ContentAnalysisResponse) -> Self {
        Self {
            start: Some(value.start),
            end: Some(value.end),
            text: Some(value.text),
            detector_id: value.detector_id,
            detection_type: value.detection_type,
            detection: value.detection,
            score: value.score,
            evidence: value
                .evidence
                .map(|vs| vs.into_iter().map(Into::into).collect())
                .unwrap_or_default(),
            metadata: value.metadata,
        }
    }
}

impl From<DetectionEvidence> for models::EvidenceObj {
    fn from(value: DetectionEvidence) -> Self {
        let evidence = (!value.evidence.is_empty())
            .then_some(value.evidence.into_iter().map(Into::into).collect());
        Self {
            name: value.name,
            value: value.value,
            score: value.score,
            evidence,
        }
    }
}

impl From<Evidence> for models::Evidence {
    fn from(value: Evidence) -> Self {
        Self {
            name: value.name,
            value: value.value,
            score: value.score,
        }
    }
}

impl From<models::EvidenceObj> for DetectionEvidence {
    fn from(value: models::EvidenceObj) -> Self {
        Self {
            name: value.name,
            value: value.value,
            score: value.score,
            evidence: value
                .evidence
                .map(|vs| vs.into_iter().map(Into::into).collect())
                .unwrap_or_default(),
        }
    }
}

impl From<models::Evidence> for Evidence {
    fn from(value: models::Evidence) -> Self {
        Self {
            name: value.name,
            value: value.value,
            score: value.score,
        }
    }
}

impl From<models::DetectionResult> for Detection {
    fn from(value: models::DetectionResult) -> Self {
        Self {
            start: None,
            end: None,
            text: None,
            detector_id: value.detector_id,
            detection_type: value.detection_type,
            detection: value.detection,
            score: value.score,
            evidence: value
                .evidence
                .map(|vs| vs.into_iter().map(Into::into).collect())
                .unwrap_or_default(),
            metadata: value.metadata,
        }
    }
}

impl From<Detection> for models::DetectionResult {
    fn from(value: Detection) -> Self {
        let evidence = (!value.evidence.is_empty())
            .then_some(value.evidence.into_iter().map(Into::into).collect());
        Self {
            detection_type: value.detection_type,
            detection: value.detection,
            detector_id: value.detector_id,
            score: value.score,
            evidence,
            metadata: value.metadata,
        }
    }
}

impl From<Detection> for models::TokenClassificationResult {
    fn from(value: Detection) -> Self {
        Self {
            start: value.start.map(|v| v as u32).unwrap(),
            end: value.end.map(|v| v as u32).unwrap(),
            word: value.text.unwrap_or_default(),
            entity: value.detection,
            entity_group: value.detection_type,
            detector_id: value.detector_id,
            score: value.score,
            token_count: None,
        }
    }
}

impl From<Detection> for detector::ContentAnalysisResponse {
    fn from(value: Detection) -> Self {
        let evidence = (!value.evidence.is_empty())
            .then_some(value.evidence.into_iter().map(Into::into).collect());
        Self {
            start: value.start.unwrap(),
            end: value.end.unwrap(),
            text: value.text.unwrap(),
            detection: value.detection,
            detection_type: value.detection_type,
            detector_id: value.detector_id,
            score: value.score,
            evidence,
            metadata: value.metadata,
        }
    }
}
