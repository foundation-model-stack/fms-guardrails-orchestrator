mod max_processed_index;
use async_trait::async_trait;
pub use max_processed_index::MaxProcessedIndexProcessor;
use tokio::sync::mpsc;

use super::streaming::DetectionResult;
use crate::models::ClassifiedGeneratedTextStreamResult;

/// Processes detection streams.
#[async_trait]
pub trait DetectionStreamProcessor: Default {
    async fn process(
        &self,
        streams: Vec<(String, mpsc::Receiver<DetectionResult>)>,
    ) -> mpsc::Receiver<ClassifiedGeneratedTextStreamResult>;
}
