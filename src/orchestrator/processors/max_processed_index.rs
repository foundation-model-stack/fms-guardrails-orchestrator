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
        let (_result_tx, result_rx) = mpsc::channel(32);
        tokio::spawn(async move {
            // loop over streams and process
            // send results to result_tx
            todo!()
        });
        result_rx
    }
}

#[cfg(test)]
mod tests {}
