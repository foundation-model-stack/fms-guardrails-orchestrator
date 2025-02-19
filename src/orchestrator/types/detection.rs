use crate::{clients::detector, models};

/// Internal representation of a single detection.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Detection {
    pub start: Option<usize>,
    pub end: Option<usize>,
    pub text: Option<String>,
    pub detector_id: Option<String>,
    pub detection_type: String,
    pub detection: String,
    pub score: f64,
    pub evidence: Vec<DetectionEvidence>,
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct DetectionEvidence {
    pub name: String,
    pub value: Option<String>,
    pub score: Option<f64>,
}

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

impl From<detector::EvidenceObj> for DetectionEvidence {
    fn from(value: detector::EvidenceObj) -> Self {
        Self {
            name: value.name,
            value: value.value,
            score: value.score,
            // TODO: evidence
        }
    }
}

impl From<models::EvidenceObj> for DetectionEvidence {
    fn from(value: models::EvidenceObj) -> Self {
        Self {
            name: value.name,
            value: value.value,
            score: value.score,
            // TODO: evidence
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
        }
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
