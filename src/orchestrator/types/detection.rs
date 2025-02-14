/// Internal representation of a single detection.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Detection {
    pub start: Option<usize>,
    pub end: Option<usize>,
    pub text: Option<String>,
    pub detection_type: String,
    pub detection: String,
    pub detector_id: Option<String>,
    pub score: f64,
    pub evidence: Vec<DetectionEvidence>,
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct DetectionEvidence {
    pub name: String,
    pub value: Option<String>,
    pub score: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Detections(Vec<Detection>);

impl Detections {
    pub fn new(values: Vec<Detection>) -> Self {
        Self(values)
    }
}

impl std::ops::Deref for Detections {
    type Target = Vec<Detection>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// Conversions
