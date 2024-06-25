mod max_processed_index;
use async_trait::async_trait;
pub use max_processed_index::MaxProcessedIndexProcessor;
use tokio::sync::mpsc;

use crate::{
    clients::detector::ContentAnalysisResponse, models::ClassifiedGeneratedTextStreamResult,
};

/// Processes detection streams.
#[async_trait]
pub trait DetectionStreamProcessor: Default {
    async fn process(
        &self,
        streams: Vec<(String, mpsc::Receiver<Vec<Vec<ContentAnalysisResponse>>>)>,
    ) -> mpsc::Receiver<ClassifiedGeneratedTextStreamResult>;
}
