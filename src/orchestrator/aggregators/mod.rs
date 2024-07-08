mod max_processed_index;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
pub use max_processed_index::MaxProcessedIndexAggregator;
use tokio::sync::mpsc;

use super::streaming::DetectionResult;
use crate::models::ClassifiedGeneratedTextStreamResult;

pub type DetectorId = String;

/// Aggregates results from detection streams.
#[async_trait]
pub trait DetectionAggregator: Default {
    async fn process(
        &self,
        generations: Arc<RwLock<Vec<ClassifiedGeneratedTextStreamResult>>>,
        detection_streams: Vec<(DetectorId, mpsc::Receiver<DetectionResult>)>,
    ) -> mpsc::Receiver<ClassifiedGeneratedTextStreamResult>;
}
