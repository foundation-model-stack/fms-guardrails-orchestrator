#![allow(dead_code)]
use std::{
    collections::{btree_map, BTreeMap},
    sync::Arc,
};

use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::instrument;

use crate::{
    models::ClassifiedGeneratedTextStreamResult,
    orchestrator::{
        streaming::{Chunk, Detections},
        Error,
    },
};

pub type DetectorId = String;
pub type Span = (i64, i64);

#[derive(Debug, Clone, Copy)]
pub enum AggregationStrategy {
    MaxProcessedIndex,
}

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
        mut generation_rx: broadcast::Receiver<ClassifiedGeneratedTextStreamResult>,
        detection_streams: Vec<(DetectorId, mpsc::Receiver<(Chunk, Detections)>)>,
    ) -> mpsc::Receiver<Result<ClassifiedGeneratedTextStreamResult, Error>> {
        // Create result channel
        let (result_tx, result_rx) = mpsc::channel(32);

        // Create actors
        let generation_actor = Arc::new(GenerationActorHandle::new());
        let result_actor = ResultActorHandle::new(generation_actor.clone(), result_tx);
        let aggregation_actor = Arc::new(AggregationActorHandle::new(
            result_actor,
            detection_streams.len(),
            //self.strategy,
        ));

        // Spawn task to send generations to generation actor
        tokio::spawn({
            async move {
                while let Ok(generation) = generation_rx.recv().await {
                    let _ = generation_actor.put(generation).await;
                }
            }
        });

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
struct ResultActorMessage {
    pub chunk: Chunk,
    pub detections: Detections,
}

/// Builds results and sends them to result channel.
struct ResultActor {
    rx: mpsc::Receiver<ResultActorMessage>,
    generation_actor: Arc<GenerationActorHandle>,
    result_tx: mpsc::Sender<Result<ClassifiedGeneratedTextStreamResult, Error>>,
}

impl ResultActor {
    pub fn new(
        rx: mpsc::Receiver<ResultActorMessage>,
        generation_actor: Arc<GenerationActorHandle>,
        result_tx: mpsc::Sender<Result<ClassifiedGeneratedTextStreamResult, Error>>,
    ) -> Self {
        Self {
            rx,
            generation_actor,
            result_tx,
        }
    }

    async fn run(&mut self) {
        while let Some(msg) = self.rx.recv().await {
            self.handle(msg).await;
        }
    }

    async fn handle(&mut self, msg: ResultActorMessage) {
        let chunk = msg.chunk;
        let detections = msg.detections;
        let generated_text: String = chunk.results.into_iter().map(|t| t.text).collect();
        let input_start_index = chunk.input_start_index as usize;
        let input_end_index = chunk.input_end_index as usize;

        // Get subset of generation responses relevant for this chunk
        let generations = self
            .generation_actor
            .get_range(input_start_index, input_end_index)
            .await;

        // Build result
        let tokens = generations
            .iter()
            .flat_map(|generation| generation.tokens.clone().unwrap_or_default())
            .collect::<Vec<_>>();
        let mut result = ClassifiedGeneratedTextStreamResult {
            generated_text: Some(generated_text),
            start_index: Some(chunk.start_index as u32),
            processed_index: Some(chunk.processed_index as u32),
            tokens: Some(tokens),
            // Populate fields from last response or default
            ..generations.last().cloned().unwrap_or_default()
        };
        result.token_classification_results.output = Some(detections);
        if input_start_index == 0 {
            // Get input_token_count and seed from first generation message
            let first = generations.first().unwrap();
            result.input_token_count = first.input_token_count;
            result.seed = first.seed;
            // Get input_tokens from second generation message (if specified)
            let input_tokens = if let Some(second) = generations.get(1) {
                second.input_tokens.clone()
            } else {
                Some(Vec::default())
            };
            result.input_tokens = input_tokens;
        }

        // Send result to result channel
        let _ = self.result_tx.send(Ok(result)).await;
    }
}

/// [`ResultActor`] handle.
struct ResultActorHandle {
    tx: mpsc::Sender<ResultActorMessage>,
}

impl ResultActorHandle {
    pub fn new(
        generation_actor: Arc<GenerationActorHandle>,
        result_tx: mpsc::Sender<Result<ClassifiedGeneratedTextStreamResult, Error>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let mut actor = ResultActor::new(rx, generation_actor, result_tx);
        tokio::spawn(async move { actor.run().await });
        Self { tx }
    }

    pub async fn send(&self, chunk: Chunk, detections: Detections) {
        let msg = ResultActorMessage { chunk, detections };
        let _ = self.tx.send(msg).await;
    }
}

#[derive(Debug)]
struct AggregationActorMessage {
    pub chunk: Chunk,
    pub detections: Detections,
}

/// Aggregates detections and sends them to [`ResultActor`].
struct AggregationActor {
    rx: mpsc::Receiver<AggregationActorMessage>,
    result_actor: ResultActorHandle,
    tracker: Tracker,
    n_detectors: usize,
}

impl AggregationActor {
    pub fn new(
        rx: mpsc::Receiver<AggregationActorMessage>,
        result_actor: ResultActorHandle,
        n_detectors: usize,
    ) -> Self {
        let tracker = Tracker::new();
        Self {
            rx,
            result_actor,
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
            // Take first span and send to result actor
            if let Some((_key, value)) = self.tracker.pop_first() {
                let chunk = value.chunk;
                let mut detections: Detections = value.detections.into_iter().flatten().collect();
                // Provide sorted detections within each chunk
                detections.sort_by_key(|r| r.start);
                let _ = self.result_actor.send(chunk, detections).await;
            }
        }
    }
}

/// [`AggregationActor`] handle.
struct AggregationActorHandle {
    tx: mpsc::Sender<AggregationActorMessage>,
}

impl AggregationActorHandle {
    pub fn new(result_actor: ResultActorHandle, n_detectors: usize) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let mut actor = AggregationActor::new(rx, result_actor, n_detectors);
        tokio::spawn(async move { actor.run().await });
        Self { tx }
    }

    pub async fn send(&self, chunk: Chunk, detections: Detections) {
        let msg = AggregationActorMessage { chunk, detections };
        let _ = self.tx.send(msg).await;
    }
}

#[derive(Debug)]
enum GenerationActorMessage {
    Put(ClassifiedGeneratedTextStreamResult),
    Get {
        index: usize,
        response_tx: oneshot::Sender<Option<ClassifiedGeneratedTextStreamResult>>,
    },
    GetRange {
        start: usize,
        end: usize,
        response_tx: oneshot::Sender<Vec<ClassifiedGeneratedTextStreamResult>>,
    },
    Length {
        response_tx: oneshot::Sender<usize>,
    },
}

/// Consumes generations from generation stream and provides them to [`ResultActor`].
struct GenerationActor {
    rx: mpsc::Receiver<GenerationActorMessage>,
    generations: Vec<ClassifiedGeneratedTextStreamResult>,
}

impl GenerationActor {
    pub fn new(rx: mpsc::Receiver<GenerationActorMessage>) -> Self {
        let generations = Vec::new();
        Self { rx, generations }
    }

    async fn run(&mut self) {
        while let Some(msg) = self.rx.recv().await {
            self.handle(msg);
        }
    }

    fn handle(&mut self, msg: GenerationActorMessage) {
        match msg {
            GenerationActorMessage::Put(generation) => self.generations.push(generation),
            GenerationActorMessage::Get { index, response_tx } => {
                let generation = self.generations.get(index).cloned();
                let _ = response_tx.send(generation);
            }
            GenerationActorMessage::GetRange {
                start,
                end,
                response_tx,
            } => {
                let generations = self.generations[start..=end].to_vec();
                let _ = response_tx.send(generations);
            }
            GenerationActorMessage::Length { response_tx } => {
                let _ = response_tx.send(self.generations.len());
            }
        }
    }
}

/// [`GenerationActor`] handle.
struct GenerationActorHandle {
    tx: mpsc::Sender<GenerationActorMessage>,
}

impl GenerationActorHandle {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(32);
        let mut actor = GenerationActor::new(rx);
        tokio::spawn(async move { actor.run().await });
        Self { tx }
    }

    pub async fn put(&self, generation: ClassifiedGeneratedTextStreamResult) {
        let msg = GenerationActorMessage::Put(generation);
        let _ = self.tx.send(msg).await;
    }

    pub async fn get(&self, index: usize) -> Option<ClassifiedGeneratedTextStreamResult> {
        let (response_tx, response_rx) = oneshot::channel();
        let msg = GenerationActorMessage::Get { index, response_tx };
        let _ = self.tx.send(msg).await;
        response_rx.await.unwrap()
    }

    pub async fn get_range(
        &self,
        start: usize,
        end: usize,
    ) -> Vec<ClassifiedGeneratedTextStreamResult> {
        let (response_tx, response_rx) = oneshot::channel();
        let msg = GenerationActorMessage::GetRange {
            start,
            end,
            response_tx,
        };
        let _ = self.tx.send(msg).await;
        response_rx.await.unwrap()
    }

    pub async fn len(&self) -> usize {
        let (response_tx, response_rx) = oneshot::channel();
        let msg = GenerationActorMessage::Length { response_tx };
        let _ = self.tx.send(msg).await;
        response_rx.await.unwrap()
    }
}

#[derive(Debug, Clone)]
struct TrackerEntry {
    pub chunk: Chunk,
    pub detections: Vec<Detections>,
}

impl TrackerEntry {
    pub fn new(chunk: Chunk, detections: Detections) -> Self {
        Self {
            chunk,
            detections: vec![detections],
        }
    }
}

#[derive(Debug, Clone)]
struct Tracker {
    state: BTreeMap<Span, TrackerEntry>,
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            state: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, key: Span, value: TrackerEntry) {
        match self.state.entry(key) {
            btree_map::Entry::Vacant(entry) => {
                // New span, insert entry with chunk and detections
                entry.insert(value);
            }
            btree_map::Entry::Occupied(mut entry) => {
                // Existing span, extend detections
                entry.get_mut().detections.extend(value.detections);
            }
        }
    }

    /// Returns the key-value pair of the first span.
    pub fn first_key_value(&self) -> Option<(&Span, &TrackerEntry)> {
        self.state.first_key_value()
    }

    /// Returns the value of the first span.
    pub fn first(&self) -> Option<&TrackerEntry> {
        self.state.first_key_value().map(|(_, value)| value)
    }

    /// Removes and returns the key-value pair of the first span.
    pub fn pop_first(&mut self) -> Option<(Span, TrackerEntry)> {
        self.state.pop_first()
    }

    /// Returns the number of elements in the tracker.
    pub fn len(&self) -> usize {
        self.state.len()
    }

    /// Returns true if the tracker contains no elements.
    pub fn is_empty(&self) -> bool {
        self.state.is_empty()
    }

    /// Gets an iterator over the keys of the tracker, in sorted order.
    pub fn keys(&self) -> btree_map::Keys<'_, (i64, i64), TrackerEntry> {
        self.state.keys()
    }

    /// Gets an iterator over the values of the tracker, in sorted order.
    pub fn values(&self) -> btree_map::Values<'_, (i64, i64), TrackerEntry> {
        self.state.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::TokenClassificationResult,
        pb::caikit_data_model::nlp::{ChunkerTokenizationStreamResult, Token},
    };

    fn get_detection_obj(
        span: Span,
        text: &str,
        detection: &str,
        detection_type: &str,
    ) -> TokenClassificationResult {
        TokenClassificationResult {
            start: span.0 as u32,
            end: span.1 as u32,
            word: text.to_string(),
            entity: detection.to_string(),
            entity_group: detection_type.to_string(),
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
            let detection = get_detection_obj(whole_span, text, "has_HAP", "HAP");
            let _ = detector_tx1.send((chunk.clone(), vec![detection])).await;

            let (detector_tx2, detector_rx2) = mpsc::channel(1);
            let detection = get_detection_obj(partial_span, text, "email_ID", "PII");
            let _ = detector_tx2.send((chunk.clone(), vec![detection])).await;

            // Push HAP after PII to make sure detection ordering is not coincidental
            detection_streams.push(("pii-1".into(), detector_rx2));
            detection_streams.push(("hap-1".into(), detector_rx1));
        }

        let (generation_tx, generation_rx) = broadcast::channel(1);
        let _ = generation_tx.send(ClassifiedGeneratedTextStreamResult::default());
        let aggregator = Aggregator::default();

        let mut result_rx = aggregator.run(generation_rx, detection_streams);
        let mut chunk_count = 0;
        while let Some(result) = result_rx.recv().await {
            let detection = result
                .unwrap()
                .token_classification_results
                .output
                .unwrap_or_default();
            assert_eq!(detection.len(), detector_count);
            // Expect HAP first since whole_span start is before partial_span start
            assert_eq!(detection[0].entity_group, "HAP");
            assert_eq!(detection[1].entity_group, "PII");
            chunk_count += 1;
        }
        assert_eq!(chunk_count, chunks.len());
    }

    #[test]
    fn test_tracker_with_out_of_order_chunks() {
        let chunks = [
            ChunkerTokenizationStreamResult {
                results: [Token {
                    start: 0,
                    end: 56,
                    text: " a powerful tool for the development \
                        of complex systems."
                        .into(),
                }]
                .to_vec(),
                token_count: 0,
                processed_index: 56,
                start_index: 0,
                input_start_index: 0,
                input_end_index: 10,
            },
            ChunkerTokenizationStreamResult {
                results: [Token {
                    start: 56,
                    end: 135,
                    text: " It has been used in many fields, such as \
                        computer vision and image processing."
                        .into(),
                }]
                .to_vec(),
                token_count: 0,
                processed_index: 135,
                start_index: 56,
                input_start_index: 11,
                input_end_index: 26,
            },
        ];
        let n_detectors = 2;
        let mut tracker = Tracker::new();

        // Insert out-of-order detection results
        for (key, value) in [
            // detector 1, chunk 2
            (
                (chunks[1].start_index, chunks[1].processed_index),
                TrackerEntry::new(chunks[1].clone(), vec![]),
            ),
            // detector 2, chunk 1
            (
                (chunks[0].start_index, chunks[0].processed_index),
                TrackerEntry::new(chunks[0].clone(), vec![]),
            ),
            // detector 2, chunk 2
            (
                (chunks[1].start_index, chunks[1].processed_index),
                TrackerEntry::new(chunks[1].clone(), vec![]),
            ),
        ] {
            tracker.insert(key, value);
        }
        // We now have both detector results for chunk 2, but not chunk 1

        // We do not have all detections for the first chunk
        assert!(
            !tracker
                .first()
                .is_some_and(|first| first.detections.len() == n_detectors),
            "detections length should not be 2 for first chunk"
        );

        // Insert entry for detector 1, chunk 1
        tracker.insert(
            (chunks[0].start_index, chunks[0].processed_index),
            TrackerEntry::new(chunks[0].clone(), vec![]),
        );

        // We have all detections for the first chunk
        assert!(
            tracker
                .first()
                .is_some_and(|first| first.detections.len() == n_detectors),
            "detections length should be 2 for first chunk"
        );

        // There should be entries for 2 chunks
        assert_eq!(tracker.len(), 2, "tracker length should be 2");

        // Detections length should be 2 for each chunk
        assert_eq!(
            tracker
                .values()
                .map(|entry| entry.detections.len())
                .collect::<Vec<_>>(),
            vec![2, 2],
            "detections length should be 2 for each chunk"
        );

        // The first entry should be for chunk 1
        let first_key = *tracker
            .first_key_value()
            .expect("tracker should have first entry")
            .0;
        assert_eq!(
            first_key,
            (chunks[0].start_index, chunks[0].processed_index),
            "first should be chunk 1"
        );

        // Tracker should remove and return entry for chunk 1
        let (key, value) = tracker
            .pop_first()
            .expect("tracker should have first entry");
        assert!(
            key.0 == 0 && value.chunk.start_index == 0,
            "first should be chunk 1"
        );
        assert!(tracker.len() == 1, "tracker length should be 1");

        // Tracker should remove and return entry for chunk 2
        let (key, value) = tracker
            .pop_first()
            .expect("tracker should have first entry");
        assert!(
            key.0 == 56 && value.chunk.start_index == 56,
            "first should be chunk 2"
        );
        assert!(tracker.is_empty(), "tracker should be empty");
    }
}
