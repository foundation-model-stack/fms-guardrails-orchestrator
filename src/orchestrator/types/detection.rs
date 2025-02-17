/// Internal representation of a single detection.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Detection {
    pub start: Option<usize>,
    pub end: Option<usize>,
    pub text: Option<String>,
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
