use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::debug;

use super::DetectionStreamProcessor;
use crate::{
    models::{ClassifiedGeneratedTextStreamResult, TextGenTokenClassificationResults},
    orchestrator::streaming::DetectionResult,
};

/// Processes detection streams applying a "max processed index" strategy.
#[derive(Default)]
pub struct MaxProcessedIndexProcessor {}

#[async_trait]
impl DetectionStreamProcessor for MaxProcessedIndexProcessor {
    async fn process(
        &self,
        generations: Arc<RwLock<Vec<ClassifiedGeneratedTextStreamResult>>>,
        streams: Vec<(String, mpsc::Receiver<DetectionResult>)>,
    ) -> mpsc::Receiver<ClassifiedGeneratedTextStreamResult> {
        let (result_tx, result_rx) = mpsc::channel(1024);
        tokio::spawn(async move {
            // TODO: implement actual aggregation logic, this is just a placeholder
            for (detector_id, mut stream) in streams {
                while let Some(result) = stream.recv().await {
                    debug!(%detector_id, "[detection_processor_task] received: {result:?}");
                    let generated_text = result.chunk.results.into_iter().map(|t| t.text).collect();
                    let detections = result
                        .detections
                        .into_iter()
                        .flat_map(|r| r.into_iter().map(Into::into))
                        .collect();
                    // TODO: figure out good approach to get details needed from generation messages.
                    // We currently pass in a shared vec behind a RwLock, but there is probably a more elegant way.
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
