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
        detection_streams: Vec<(DetectorId, f64, mpsc::Receiver<DetectionResult>)>,
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

            for (detector_id, threshold, mut stream) in detection_streams {
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
                            r.into_iter().filter_map(|resp| {
                                let result: TokenClassificationResult = resp.into();
                                (result.score >= threshold).then_some(result)
                            })
                        })
                        .collect();

                    let input_start_index = chunk.input_start_index as usize;
                    let input_end_index = chunk.input_end_index as usize;

                    // Get subset of generation responses relevant for this chunk
                    let generation_responses: Vec<ClassifiedGeneratedTextStreamResult> =
                        generations.read().unwrap()[input_start_index..=input_end_index]
                            .iter()
                            .map(|result| result.to_owned())
                            .collect::<Vec<_>>();

                    let tokens = generation_responses
                        .iter()
                        .flat_map(|result| result.tokens.clone().unwrap_or([].to_vec()))
                        .collect::<Vec<_>>();

                    let mut result: ClassifiedGeneratedTextStreamResult =
                        ClassifiedGeneratedTextStreamResult {
                            generated_text: Some(generated_text.clone()),
                            start_index: chunk.start_index as u32,
                            processed_index: Some(chunk.processed_index as u32),
                            tokens: Some(tokens),
                            // Populate all fields from last generation response and if not available, then use
                            // default value for ClassifiedGeneratedTextStreamResult
                            ..generation_responses
                                .last()
                                .unwrap_or(&ClassifiedGeneratedTextStreamResult::default())
                                .to_owned()
                        };

                    // input_token_count and input_tokens to be only present in 1st output
                    // seed to be present in 1st and last output. These are in accordance with how TGIS (provider) returns output
                    // seed will automatically get into last from above logic of using `..generation_responses.last`
                    if (input_start_index..input_end_index).contains(&0) {
                        // Note we need to optimize below a bit and only read generations 1 time above this loop
                        let initial_gen_response = generations.read().unwrap()[0].clone();

                        let input_token_count = initial_gen_response.input_token_count;
                        let seed = initial_gen_response.seed;
                        result.input_token_count = input_token_count;
                        result.seed = seed;
                    } else if (input_start_index..input_end_index).contains(&1) {
                        // Note: input_tokens is not present in 0th response, so we use `1`
                        let input_tokens = match generations.read().unwrap().get(1) {
                            Some(first_generation) => first_generation.input_tokens.clone(),
                            None => Some([].to_vec()),
                        };
                        result.input_tokens = input_tokens;
                    }

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
        clients::detector::ContentAnalysisResponse, pb::caikit::runtime::chunkers,
        pb::caikit_data_model::nlp::Token,
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
        let mut chunks: Vec<chunkers::ChunkerTokenizationStreamResult> = [].into();

        chunks.push(chunkers::ChunkerTokenizationStreamResult {
            results: [Token {
                start: 0,
                end: 24,
                text: "This is a dummy sentence".into(),
            }]
            .into(),
            token_count: 5,
            processed_index: 4,
            start_index: 0,
            input_start_index: 0,
            input_end_index: 0,
        });

        let detector_count = 2;
        let mut detection_streams = Vec::with_capacity(detector_count);

        // Note: below is detection / chunks on batch of size 1 with 1 sentence
        for chunk in &chunks {
            let chunk_token = chunk.results[0].clone();
            let text = &chunk_token.text;
            let span = (chunk_token.start as u32, chunk_token.end as u32);
            let threshold = 0.001;
            // Add multiple detections to same chunk

            let (detector_tx1, detector_rx1) = mpsc::channel(1024);
            let detector_response = get_detection_obj(span, text, "has_HAP", "HAP");
            let detector_id = String::from("hap-1");
            let detection_result =
                DetectionResult::new(chunk.clone(), [detector_response].to_vec());
            let _ = detector_tx1.send(detection_result).await;
            detection_streams.push((detector_id, threshold, detector_rx1));

            let (detector_tx2, detector_rx2) = mpsc::channel(1024);
            let detector_response = get_detection_obj(span, text, "email_ID", "PII");
            let detector_id = String::from("pii-1");
            let detection_result =
                DetectionResult::new(chunk.clone(), [detector_response].to_vec());
            let _ = detector_tx2.send(detection_result).await;
            detection_streams.push((detector_id, threshold, detector_rx2));
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
