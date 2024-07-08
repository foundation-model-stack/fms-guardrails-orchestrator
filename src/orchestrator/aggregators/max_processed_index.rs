use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::debug;

use super::{DetectionAggregator, DetectorId};
use crate::{
    models::{ClassifiedGeneratedTextStreamResult, TextGenTokenClassificationResults},
    orchestrator::streaming::DetectionResult,
};

/// Aggregates results applying a "max processed index" strategy.
#[derive(Default)]
pub struct MaxProcessedIndexAggregator {}

#[async_trait]
impl DetectionAggregator for MaxProcessedIndexAggregator {
    async fn process(
        &self,
        generations: Arc<RwLock<Vec<ClassifiedGeneratedTextStreamResult>>>,
        detection_streams: Vec<(DetectorId, mpsc::Receiver<DetectionResult>)>,
    ) -> mpsc::Receiver<ClassifiedGeneratedTextStreamResult> {
        let (result_tx, result_rx) = mpsc::channel(1024);
        tokio::spawn(async move {
            // TODO:
            // - Implement actual aggregation logic, this is just a placeholder
            // - Figure out good approach to get details needed from generation messages (using shared vec for now)
            // - Apply thresholds
            // - TBD
            for (detector_id, mut stream) in detection_streams {
                while let Some(result) = stream.recv().await {
                    debug!(%detector_id, ?result, "[detection_processor_task] received detection result");
                    let generated_text = result.chunk.results.into_iter().map(|t| t.text).collect();
                    let detections = result
                        .detections
                        .into_iter()
                        .flat_map(|r| r.into_iter().map(Into::into))
                        .collect();
                    let input_token_count = generations.read().unwrap()[0].input_token_count;
                    let result = ClassifiedGeneratedTextStreamResult {
                        generated_text: Some(generated_text),
                        //finish_reason:
                        input_token_count,
                        //generated_token_count:
                        //seed:
                        start_index: result.chunk.start_index as u32,
                        processed_index: Some(result.chunk.processed_index as u32),
                        token_classification_results: TextGenTokenClassificationResults {
                            input: None,
                            output: Some(detections),
                        },
                        ..Default::default()
                    };
                    let _ = result_tx.send(result).await;
                }
            }
        });
        result_rx
    }
}

#[cfg(test)]
mod tests {}
