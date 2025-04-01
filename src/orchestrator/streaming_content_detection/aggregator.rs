/*
 Copyright FMS Guardrails Orchestrator Authors

 Licensed under the Apache License, Version 2.0 (the "License");
 you may not use this file except in compliance with the License.
 You may obtain a copy of the License at

     http://www.apache.org/licenses/LICENSE-2.0

 Unless required by applicable law or agreed to in writing, software
 distributed under the License is distributed on an "AS IS" BASIS,
 WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 See the License for the specific language governing permissions and
 limitations under the License.

*/

//////////////////////////////////////////////////////////////////////////////////////
// This file contains a simplified version of the code in                           //
// src/orchestrator/streaming/aggregator.rs.                                        //
// The main difference is that this file does not contain generation nor a          //
// result actor. This code also reuses whatever is possible from the aforementioned //
// file, such as tracker and aggregation strategy structs.                          //
// `ClassifiedGeneratedTextStreamResult` with `StreamingContentDetectionRequest`    //
// and `StreamingContentDetectionResponse`.                                         //
// This can likely be improved in a future refactor to use generics instead of      //
// duplicating these very similar methods.                                          //
//////////////////////////////////////////////////////////////////////////////////////
#![allow(dead_code)]
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::instrument;

use crate::{
    clients::detector::ContentAnalysisResponse,
    models::{Metadata, StreamingContentDetectionResponse},
    orchestrator::{
        Error,
        streaming::{
            Chunk, Detections,
            aggregator::{AggregationStrategy, DetectorId, Tracker, TrackerEntry},
        },
    },
};

pub struct Aggregator {
    strategy: AggregationStrategy,
}

impl Default for Aggregator {
    fn default() -> Self {
        Self {
            strategy: AggregationStrategy::MaxProcessedIndex,
        }
    }
}

impl Aggregator {
    pub fn new(strategy: AggregationStrategy) -> Self {
        Self { strategy }
    }

    #[instrument(skip_all)]
    pub fn run(
        &self,
        detection_streams: Vec<(DetectorId, mpsc::Receiver<(Chunk, Detections)>)>,
    ) -> mpsc::Receiver<Result<StreamingContentDetectionResponse, Error>> {
        // Create result channel
        let (result_tx, result_rx) = mpsc::channel(32);

        // Create actors
        let aggregation_actor = Arc::new(AggregationActorHandle::new(
            result_tx,
            detection_streams.len(),
        ));

        // Spawn tasks to process detection streams concurrently
        for (_detector_id, mut stream) in detection_streams {
            let aggregation_actor = aggregation_actor.clone();
            tokio::spawn(async move {
                while let Some((chunk, detections)) = stream.recv().await {
                    // Send to aggregation actor
                    aggregation_actor.send(chunk, detections).await;
                }
            });
        }
        result_rx
    }
}

#[derive(Debug)]
struct AggregationActorMessage {
    pub chunk: Chunk,
    pub detections: Detections,
}

/// Aggregates detections, builds results, and sends them to result channel.
struct AggregationActor {
    rx: mpsc::Receiver<AggregationActorMessage>,
    result_tx: mpsc::Sender<Result<StreamingContentDetectionResponse, Error>>,
    tracker: Tracker,
    n_detectors: usize,
}

impl AggregationActor {
    pub fn new(
        rx: mpsc::Receiver<AggregationActorMessage>,
        result_tx: mpsc::Sender<Result<StreamingContentDetectionResponse, Error>>,
        n_detectors: usize,
    ) -> Self {
        let tracker = Tracker::new();
        Self {
            rx,
            result_tx,
            tracker,
            n_detectors,
        }
    }

    async fn run(&mut self) {
        while let Some(msg) = self.rx.recv().await {
            self.handle(msg).await;
        }
    }

    async fn handle(&mut self, msg: AggregationActorMessage) {
        let chunk = msg.chunk;
        let detections = msg.detections;

        // Add to tracker
        let span = (chunk.start_index, chunk.processed_index);
        self.tracker
            .insert(span, TrackerEntry::new(chunk, detections));

        // Check if we have all detections for the first span
        if self
            .tracker
            .first()
            .is_some_and(|first| first.detections.len() == self.n_detectors)
        {
            // Take first span and send result
            if let Some((_key, value)) = self.tracker.pop_first() {
                let chunk = value.chunk;
                let mut detections: Detections = value.detections.into_iter().flatten().collect();
                // Provide sorted detections within each chunk
                detections.sort_by_key(|r| r.start);

                // Build response message
                let response = StreamingContentDetectionResponse {
                    start_index: chunk.start_index as u32,
                    processed_index: chunk.processed_index as u32,
                    detections: detections
                        .into_iter()
                        .map(|r| ContentAnalysisResponse {
                            start: r.start as usize,
                            end: r.end as usize,
                            text: r.word,
                            detection: r.entity,
                            detection_type: r.entity_group,
                            detector_id: r.detector_id,
                            score: r.score,
                            evidence: None,
                            metadata: Metadata::new(),
                        })
                        .collect(),
                };
                // Send to result channel
                let _ = self.result_tx.send(Ok(response)).await;
            }
        }
    }
}

/// [`AggregationActor`] handle.
struct AggregationActorHandle {
    tx: mpsc::Sender<AggregationActorMessage>,
}

impl AggregationActorHandle {
    pub fn new(
        result_tx: mpsc::Sender<Result<StreamingContentDetectionResponse, Error>>,
        n_detectors: usize,
    ) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let mut actor = AggregationActor::new(rx, result_tx, n_detectors);
        tokio::spawn(async move { actor.run().await });
        Self { tx }
    }

    pub async fn send(&self, chunk: Chunk, detections: Detections) {
        let msg = AggregationActorMessage { chunk, detections };
        let _ = self.tx.send(msg).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::TokenClassificationResult, orchestrator::streaming::aggregator::Span,
        pb::caikit_data_model::nlp::Token,
    };

    fn get_detection_obj(
        span: Span,
        text: &str,
        detection: &str,
        detection_type: &str,
        detector_id: &str,
    ) -> TokenClassificationResult {
        TokenClassificationResult {
            start: span.0 as u32,
            end: span.1 as u32,
            word: text.to_string(),
            entity: detection.to_string(),
            entity_group: detection_type.to_string(),
            detector_id: Some(detector_id.to_string()),
            score: 0.99,
            token_count: None,
        }
    }

    #[tokio::test]
    /// Test to check the aggregation of streaming generation results with multiple detectors on a single chunk.
    async fn test_aggregation_single_chunk_multi_detection() {
        let chunks = vec![Chunk {
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
        }];

        let detector_count = 2;
        let mut detection_streams = Vec::with_capacity(detector_count);

        // Note: below is detection / chunks on batch of size 1 with 1 sentence
        for chunk in &chunks {
            let chunk_token = chunk.results[0].clone();
            let text = &chunk_token.text;
            let whole_span = (chunk_token.start, chunk_token.end);
            let partial_span = (chunk_token.start + 2, chunk_token.end - 2);

            let (detector_tx1, detector_rx1) = mpsc::channel(1);
            let detection = get_detection_obj(whole_span, text, "has_HAP", "HAP", "en-hap");
            let _ = detector_tx1.send((chunk.clone(), vec![detection])).await;

            let (detector_tx2, detector_rx2) = mpsc::channel(1);
            let detection = get_detection_obj(partial_span, text, "email_ID", "PII", "en-pii");
            let _ = detector_tx2.send((chunk.clone(), vec![detection])).await;

            // Push HAP after PII to make sure detection ordering is not coincidental
            detection_streams.push(("pii-1".into(), detector_rx2));
            detection_streams.push(("hap-1".into(), detector_rx1));
        }

        let aggregator = Aggregator::new(AggregationStrategy::MaxProcessedIndex);
        let mut result_rx = aggregator.run(detection_streams);
        let mut chunk_count = 0;
        while let Some(result) = result_rx.recv().await {
            let detection = result.unwrap().detections;
            assert_eq!(detection.len(), detector_count);
            // Expect HAP first since whole_span start is before partial_span start
            assert_eq!(detection[0].detection_type, "HAP");
            assert_eq!(detection[1].detection_type, "PII");
            chunk_count += 1;
        }
        assert_eq!(chunk_count, chunks.len());
    }
}
