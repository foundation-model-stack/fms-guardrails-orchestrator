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
use crate::{clients::detector, models};

/// A detection.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Detection {
    /// Start index of the detection
    pub start: Option<usize>,
    /// End index of the detection
    pub end: Option<usize>,
    /// Text corresponding to the detection
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
    pub evidence: Vec<DetectionEvidence>,
    /// Detection metadata
    pub metadata: models::Metadata,
}

/// Detection evidence.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct DetectionEvidence {
    /// Evidence name
    pub name: String,
    /// Evidence value
    pub value: Option<String>,
    /// Evidence score
    pub score: Option<f64>,
    /// Additional evidence
    pub evidence: Vec<Evidence>,
}

/// Additional detection evidence.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Evidence {
    pub name: String,
    pub value: Option<String>,
    pub score: Option<f64>,
}

/// An array of detections.
#[derive(Default, Debug, Clone)]
pub struct Detections(Vec<Detection>);

impl Detections {
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::ops::Deref for Detections {
    type Target = Vec<Detection>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Detections {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoIterator for Detections {
    type Item = Detection;
    type IntoIter = <Vec<Detection> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<Detection> for Detections {
    fn from_iter<T: IntoIterator<Item = Detection>>(iter: T) -> Self {
        let mut detections = Detections::new();
        for value in iter {
            detections.push(value);
        }
        detections
    }
}

impl From<Vec<Detection>> for Detections {
    fn from(value: Vec<Detection>) -> Self {
        Self(value)
    }
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

impl From<Vec<Vec<detector::ContentAnalysisResponse>>> for Detections {
    fn from(value: Vec<Vec<detector::ContentAnalysisResponse>>) -> Self {
        value
            .into_iter()
            .flatten()
            .map(|detection| detection.into())
            .collect::<Detections>()
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

impl From<Detections> for Vec<models::DetectionResult> {
    fn from(value: Detections) -> Self {
        value.into_iter().map(Into::into).collect()
    }
}

impl From<Vec<models::DetectionResult>> for Detections {
    fn from(value: Vec<models::DetectionResult>) -> Self {
        value.into_iter().map(Into::into).collect()
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

impl From<Detections> for Vec<models::TokenClassificationResult> {
    fn from(value: Detections) -> Self {
        value.into_iter().map(Into::into).collect()
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

impl From<Detections> for Vec<detector::ContentAnalysisResponse> {
    fn from(value: Detections) -> Self {
        value.into_iter().map(Into::into).collect()
    }
}
