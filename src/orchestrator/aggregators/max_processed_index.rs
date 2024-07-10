use std::{
    borrow::{Borrow, BorrowMut},
    collections::{BTreeMap, HashMap},
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::debug;

use super::{DetectionAggregator, DetectorId};
use crate::{
    models::{
        ClassifiedGeneratedTextStreamResult, TextGenTokenClassificationResults,
        TokenClassificationResult,
    },
    orchestrator::streaming::DetectionResult,
};

/// Aggregates results applying a "max processed index" strategy.
#[derive(Default)]
pub struct MaxProcessedIndexAggregator {}

trait AddDetectionResult {
    fn add_detection_result(
        &mut self,
        start: u32,
        end: u32,
        new_detection_results: Vec<TokenClassificationResult>,
        classified_stream_result: ClassifiedGeneratedTextStreamResult,
    );

    fn find_first(&self, start: u32) -> Option<(u32, u32)>;
}

impl AddDetectionResult for BTreeMap<(u32, u32), (ClassifiedGeneratedTextStreamResult, usize)> {
    /// Adds detection results to the aggregator.
    ///
    /// # Arguments
    /// * `start` - The starting index of the detection results.
    /// * `end` - The ending index of the detection results.
    /// * `new_detection_results` - The new detection results to add.
    /// * `classified_stream_result` - The classified stream result associated with these detection results.
    fn add_detection_result(
        &mut self,
        start: u32,
        end: u32,
        new_detection_results: Vec<TokenClassificationResult>,
        classified_stream_result: ClassifiedGeneratedTextStreamResult,
    ) {
        // NOTE: below logic is assuming that 1 detection will only return 1 result for 1 span
        // NOTE: below logic currently is assuming 1 type of chunking for all detectors. We
        // need to expand this to have the possibility that spans can overlap, in which case
        // we would change the spans stored in the tree.

        // Check if index exist in the BTreeMap
        // If spans does not exist, insert it with the provided detection results and count of 1
        // if they do exist, then increment number of detector count and insert additional
        // detector in output vector.
        self.entry((start, end))
            .and_modify(|(old_classified_stream_result, num)| {
                old_classified_stream_result
                    .token_classification_results
                    .output = Some(new_detection_results);
                *num += 1;
            })
            .or_insert_with(|| (classified_stream_result.clone(), 1));
    }

    /// Finds the first available span in the BTreeMap.
    fn find_first(&self, start: u32) -> Option<(u32, u32)> {
        for (key, _) in self.iter() {
            if key.0 == start {
                return Some(key.clone());
            }
        }
        None
    }
}

#[async_trait]
impl DetectionAggregator for MaxProcessedIndexAggregator {
    async fn process(
        &self,
        generations: Arc<RwLock<Vec<ClassifiedGeneratedTextStreamResult>>>,
        detection_streams: Vec<(DetectorId, mpsc::Receiver<DetectionResult>)>,
    ) -> mpsc::Receiver<ClassifiedGeneratedTextStreamResult> {
        let (result_tx, result_rx) = mpsc::channel(1024);
        tokio::spawn(async move {
            // TODO: Add chunker type

            let mut processed_index = 0;

            let total_detectors: usize = detection_streams.len();
            // We use BTreeMap since it is ordered and automatically keeps all the information sorted
            // We map spans with tuple of classifiedGeneratedTextStreamResult and count of detectors already applied
            // Later on we can change this tuple of a struct for better management and cleanliness
            let mut detection_tracker: BTreeMap<
                (u32, u32),
                (ClassifiedGeneratedTextStreamResult, usize),
            > = std::collections::BTreeMap::new();

            // TODO:
            // - Implement actual aggregation logic, this is just a placeholder
            // - Figure out good approach to get details needed from generation messages (using shared vec for now)
            // - Apply thresholds
            // - TBD

            for (detector_id, mut stream) in detection_streams {
                while let Some(result) = stream.recv().await {
                    // NOTE: We expect the detector to respond with an answer, even if it is [] in case of no detections. example PII

                    debug!(%detector_id, ?result, "[detection_processor_task] received detection result");
                    let generated_text: String =
                        result.chunk.results.into_iter().map(|t| t.text).collect();
                    let detections: Vec<TokenClassificationResult> = result
                        .detections
                        .into_iter()
                        .flat_map(|r| {
                            r.into_iter().map(|mut detection| {
                                detection.start += result.chunk.start_index as usize;
                                detection.end += result.chunk.start_index as usize;
                                detection.into()
                            })
                        })
                        .collect();

                    let input_token_count = generations.read().unwrap()[0].input_token_count;

                    let classification_result = ClassifiedGeneratedTextStreamResult {
                        generated_text: Some(generated_text.clone()),
                        // TODO: Populate following generation stream fields
                        // finish_reason,
                        input_token_count,
                        // generated_token_count,
                        // seed,
                        start_index: result.chunk.start_index as u32,
                        processed_index: Some(result.chunk.processed_index as u32),
                        ..Default::default()
                    };

                    // TODO: Remove clone from `detections`
                    detection_tracker.add_detection_result(
                        result.chunk.start_index as u32,
                        result.chunk.processed_index as u32,
                        detections.clone(),
                        classification_result,
                    );

                    if processed_index == 0 && !detection_tracker.is_empty() {
                        // Nothing has been sent. Consider check for chunk starting at 0 in detection_tracker
                        // Since BTreeMap are sorted, we can rely on 1st element in detection_tracker to be the 1st one we
                        // want to send
                        let (span, (classified_result, num_detectors)) =
                            detection_tracker.first_key_value().unwrap();
                        // Check if all detectors have responded for this detector
                        if num_detectors.to_owned() == total_detectors {
                            let _ = result_tx.send(classified_result.clone()).await;
                            // Make processed_index as the end of the detected span
                            processed_index = span.1;
                            // TODO: At this point we can remove the 1st element from the detection_tracker
                            // and simplify entirity of this if-condition. But keeping it as is for now,
                            // since this information can be useful in future for handling different edge-cases
                            // and different types of chunkers
                        }
                    } else if processed_index > 0 {
                        // We are in the middle of streaming and processed_index is non-zero
                        let span = detection_tracker.find_first(processed_index);
                        // Spans may not be found in certain cases, like if we have exhausted the stream.
                        if span.is_some() {
                            let span = span.unwrap();
                            // spans found.
                            let (classified_result, num_detectors) =
                                detection_tracker.get(&span).unwrap();
                            if num_detectors.to_owned() == total_detectors {
                                let _ = result_tx.send(classified_result.clone()).await;
                                // Make processed_index as the end of the detected span
                                processed_index = span.1;
                            }
                        }
                    }
                }
            }
        });
        result_rx
    }
}

#[cfg(test)]
mod tests {

    use crate::{
        clients::detector::ContentAnalysisResponse,
        pb::caikit_data_model::nlp::{Token, TokenizationStreamResult},
    };

    use super::*;
    use std::{
        collections::HashMap,
        pin::Pin,
        sync::{Arc, RwLock},
    };
    use tokio::sync::{
        broadcast,
        mpsc::{Receiver, Sender},
    };

    async fn get_dummy_streaming_generation(
    ) -> Arc<RwLock<Vec<ClassifiedGeneratedTextStreamResult>>> {
        let dummy_result = Arc::new(RwLock::new(Vec::new()));

        dummy_result
            .write()
            .unwrap()
            .push(ClassifiedGeneratedTextStreamResult::default());

        return dummy_result;
    }

    async fn get_dummy_detection_stream(
        detector_len: usize,
        detector_tx: mpsc::Sender<DetectionResult>,
        chunks: Vec<TokenizationStreamResult>,
    ) -> Vec<(DetectorId, mpsc::Receiver<DetectionResult>)> {
        let mut detection_streams = Vec::with_capacity(detector_len);

        // Note: below is detection / chunks on batch of size 1 with 1 sentence
        for chunk in chunks {
            let detector_response: Vec<Vec<ContentAnalysisResponse>> =
                [[ContentAnalysisResponse {
                    start: 0,
                    end: 24,
                    text: "This is a dummy sentence".to_string(),
                    detection: "has_HAP".to_string(),
                    detection_type: "HAP".to_string(),
                    score: 0.99,
                    evidences: None,
                }]
                .to_vec()]
                .to_vec();
            let detection_result = DetectionResult::new(chunk, detector_response);
            let _ = detector_tx.send(detection_result).await;
        }

        return detection_streams;
    }

    #[tokio::test]
    async fn test_aggregation_single_input() {
        let (detector_tx, detector_rx) = mpsc::channel(1024);
        // Create chunks
        let mut chunks: Vec<TokenizationStreamResult> = [].into();

        chunks.push(TokenizationStreamResult {
            results: [Token {
                start: 0,
                end: 24,
                text: "This is a dummy sentence".into(),
            }]
            .into(),
            token_count: 5,
            processed_index: 4,
            start_index: 0,
        });

        let detection_stream = get_dummy_detection_stream(1, detector_tx, chunks).await;
        let generations = get_dummy_streaming_generation().await;
        let aggregator = MaxProcessedIndexAggregator::default();

        let result = aggregator.process(generations, detection_stream).await;
    }
}
