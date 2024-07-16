use std::{
    collections::{btree_map, BTreeMap},
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::debug;

use super::{DetectionAggregator, DetectorId};
use crate::{
    models::{ClassifiedGeneratedTextStreamResult, TokenClassificationResult},
    orchestrator::streaming::DetectionResult,
};

/// Aggregates results applying a "max processed index" strategy.
#[derive(Default)]
pub struct MaxProcessedIndexAggregator {}

type Span = (u32, u32);
struct DetectionTracker(BTreeMap<Span, (ClassifiedGeneratedTextStreamResult, usize)>);

impl DetectionTracker {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn insert(
        &mut self,
        span: Span,
        detections: Vec<TokenClassificationResult>,
        mut result: ClassifiedGeneratedTextStreamResult,
    ) {
        // NOTES:
        // 1. Assumes 1 detection will return 1 result for 1 span
        // 2. Assumes same chunker type is used by all detectors
        // 3. Needs to be expanded to support overlapping spans

        // Insert new or update existing entry
        let entry = self.0.entry(span);
        match entry {
            btree_map::Entry::Vacant(e) => {
                // Add detections to result
                result.token_classification_results.output = Some(detections);

                // Insert result, set detector count to 1
                e.insert((result, 1));
            }
            btree_map::Entry::Occupied(mut e) => {
                // Get existing result
                let (result, num_detectors) = e.get_mut();

                // Add detections to existing result
                if let Some(existing_detections) =
                    result.token_classification_results.output.as_mut()
                {
                    existing_detections.extend(detections);
                }

                // Increment detector count
                *num_detectors += 1;
            }
        }
    }

    pub fn find_by_span_start(
        &self,
        start: u32,
    ) -> Option<&(ClassifiedGeneratedTextStreamResult, usize)> {
        self.0
            .iter()
            .find(|(span, _)| span.0 == start)
            .map(|(_, value)| value)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn first_key_value(
        &self,
    ) -> Option<(&Span, &(ClassifiedGeneratedTextStreamResult, usize))> {
        self.0.first_key_value()
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

            let total_detectors = detection_streams.len();
            // We use BTreeMap since it is ordered and automatically keeps all the information sorted
            // We map spans with tuple of classifiedGeneratedTextStreamResult and count of detectors already applied
            // Later on we can change this tuple of a struct for better management and cleanliness
            let mut detection_tracker = DetectionTracker::new();

            // TODO:
            // - Implement actual aggregation logic, this is just a placeholder
            // - Figure out good approach to get details needed from generation messages (using shared vec for now)
            // - Apply thresholds
            // - TBD

            for (detector_id, mut stream) in detection_streams {
                while let Some(message) = stream.recv().await {
                    debug!(%detector_id, ?message, "[detection_processor_task] received detection message");
                    // NOTE: We expect the detector to respond with an answer, even if it is [] in case of no detections. example PII
                    let chunk = message.chunk;
                    let detections = message.detections;
                    let generated_text: String =
                        chunk.results.into_iter().map(|t| t.text).collect();
                    let detections: Vec<TokenClassificationResult> = detections
                        .into_iter()
                        .flat_map(|r| {
                            r.into_iter().map(|mut detection| {
                                detection.start += chunk.start_index as usize;
                                detection.end += chunk.start_index as usize;
                                detection.into()
                            })
                        })
                        .collect();

                    let input_token_count = generations.read().unwrap()[0].input_token_count;

                    let result = ClassifiedGeneratedTextStreamResult {
                        generated_text: Some(generated_text.clone()),
                        // TODO: Populate following generation stream fields
                        // finish_reason,
                        input_token_count,
                        // generated_token_count,
                        // seed,
                        start_index: chunk.start_index as u32,
                        processed_index: Some(chunk.processed_index as u32),
                        ..Default::default()
                    };

                    let span: Span = (chunk.start_index as u32, chunk.processed_index as u32);

                    detection_tracker.insert(span, detections, result);

                    if processed_index == 0 && !detection_tracker.is_empty() {
                        // Nothing has been sent. Consider check for chunk starting at 0 in detection_tracker
                        // Since BTreeMap are sorted, we can rely on 1st element in detection_tracker to be the 1st one we
                        // want to send
                        let (span, (result, num_detectors)) =
                            detection_tracker.first_key_value().unwrap();
                        // Check if all detectors have responded for this detector
                        if *num_detectors == total_detectors {
                            let _ = result_tx.send(result.clone()).await;
                            // Make processed_index as the end of the detected span
                            processed_index = span.1;
                            // TODO: At this point we can remove the 1st element from the detection_tracker
                            // and simplify entirity of this if-condition. But keeping it as is for now,
                            // since this information can be useful in future for handling different edge-cases
                            // and different types of chunkers
                        }
                    } else if processed_index > 0 {
                        // We are in the middle of streaming and processed_index is non-zero
                        if let Some((result, num_detectors)) =
                            detection_tracker.find_by_span_start(processed_index)
                        {
                            // spans found.
                            if *num_detectors == total_detectors {
                                let _ = result_tx.send(result.clone()).await;
                                // Make processed_index as the end of the detected span
                                processed_index = result.processed_index.unwrap();
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
    use std::sync::{Arc, RwLock};

    async fn get_dummy_streaming_generation(
    ) -> Arc<RwLock<Vec<ClassifiedGeneratedTextStreamResult>>> {
        let dummy_result = Arc::new(RwLock::new(Vec::new()));

        dummy_result
            .write()
            .unwrap()
            .push(ClassifiedGeneratedTextStreamResult::default());
        dummy_result
    }

    fn get_detection_obj(
        span: Span,
        text: &String,
        detection: &str,
        detection_type: &str,
    ) -> Vec<ContentAnalysisResponse> {
        [ContentAnalysisResponse {
            start: span.0 as usize,
            end: span.1 as usize,
            text: text.to_string(),
            detection: detection.to_string(),
            detection_type: detection_type.to_string(),
            score: 0.99,
            evidences: None,
        }]
        .to_vec()
    }

    #[tokio::test]
    /// Test to check the aggregation of streaming generation results with multiple detectors on a single chunk.
    async fn test_aggregation_single_chunk_multi_detection() {
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

        let detector_count = 2;
        let mut detection_streams = Vec::with_capacity(detector_count);

        // Note: below is detection / chunks on batch of size 1 with 1 sentence
        for chunk in &chunks {
            let chunk_token = chunk.results[0].clone();
            let text = &chunk_token.text;
            let span = (chunk_token.start as u32, chunk_token.end as u32);
            // Add multiple detections to same chunk

            let (detector_tx1, detector_rx1) = mpsc::channel(1024);
            let detector_response = get_detection_obj(span, text, "has_HAP", "HAP");
            let detector_id = String::from("hap-1");
            let detection_result =
                DetectionResult::new(chunk.clone(), [detector_response].to_vec());
            let _ = detector_tx1.send(detection_result).await;
            detection_streams.push((detector_id, detector_rx1));

            let (detector_tx2, detector_rx2) = mpsc::channel(1024);
            let detector_response = get_detection_obj(span, text, "email_ID", "PII");
            let detector_id = String::from("pii-1");
            let detection_result =
                DetectionResult::new(chunk.clone(), [detector_response].to_vec());
            let _ = detector_tx2.send(detection_result).await;
            detection_streams.push((detector_id, detector_rx2));
        }

        let generations = get_dummy_streaming_generation().await;
        let aggregator = MaxProcessedIndexAggregator::default();

        let mut result_rx = aggregator.process(generations, detection_streams).await;

        let mut chunk_count = 0;
        while let Some(classified_gen_stream_result) = result_rx.recv().await {
            let detection = classified_gen_stream_result
                .token_classification_results
                .output
                .unwrap_or(Vec::new());
            assert_eq!(detection.len(), detector_count);
            assert_eq!(detection[0].entity_group, "HAP");
            assert_eq!(detection[1].entity_group, "PII");
            chunk_count += 1;
        }
        assert_eq!(chunk_count, chunks.len());
    }
}
