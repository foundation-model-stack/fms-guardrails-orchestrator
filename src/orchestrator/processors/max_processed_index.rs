use async_trait::async_trait;
use tokio::sync::mpsc;

use super::DetectionStreamProcessor;
use crate::{
    clients::detector::ContentAnalysisResponse, models::ClassifiedGeneratedTextStreamResult,
};

/// Processes detection streams applying a "max processed index" strategy.
#[derive(Default)]
pub struct MaxProcessedIndexProcessor {}

#[async_trait]
impl DetectionStreamProcessor for MaxProcessedIndexProcessor {
    async fn process(
        &self,
        _streams: Vec<(String, mpsc::Receiver<Vec<Vec<ContentAnalysisResponse>>>)>, // (detector_id, detection_stream)
    ) -> mpsc::Receiver<ClassifiedGeneratedTextStreamResult> {
        todo!()
    }
}

#[cfg(test)]
mod tests {}
