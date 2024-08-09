#![allow(dead_code)]
use std::sync::Arc;

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
        let (result_tx, result_rx) = mpsc::channel(1024);

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
struct ResultActorMessage((Chunk, Detections));

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
        let (chunk, detections) = msg.0;
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
            generated_text: Some(generated_text.clone()),
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
        let (tx, rx) = mpsc::channel(8);
        let mut actor = ResultActor::new(rx, generation_actor, result_tx);
        tokio::spawn(async move { actor.run().await });
        Self { tx }
    }

    pub async fn send(&self, chunk: Chunk, detections: Detections) {
        let msg = ResultActorMessage((chunk, detections));
        let _ = self.tx.send(msg).await;
    }
}

#[derive(Debug)]
struct AggregationActorMessage {
    pub chunk: Chunk,
    pub detections: Detections,
}

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
        // TODO: support overlapping spans from different chunkers?
        let chunk = msg.chunk;
        let detections = msg.detections;

        // Add to tracker
        let span: Span = (chunk.start_index, chunk.processed_index);
        self.tracker.push((span, detections));

        // Find current detections for this span
        let current = self.tracker.find_by_span_start(chunk.start_index);

        //debug!(?self.tracker, "tracker snapshot");

        // If we have results from all detectors, send to result actor
        if current.len() == self.n_detectors {
            let detections = self
                .tracker
                .take(current)
                .into_iter()
                .flat_map(|(_, detections)| detections)
                .collect::<Vec<_>>();
            let _ = self.result_actor.send(chunk, detections).await;
        }
    }
}

/// [`AggregationActor`] handle.
struct AggregationActorHandle {
    tx: mpsc::Sender<AggregationActorMessage>,
}

impl AggregationActorHandle {
    pub fn new(
        result_actor: ResultActorHandle,
        n_detectors: usize,
        //strategy: AggregationStrategy,
    ) -> Self {
        let (tx, rx) = mpsc::channel(8);
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
        let (tx, rx) = mpsc::channel(8);
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
struct Tracker {
    state: Vec<Option<(Span, Detections)>>,
}

impl Tracker {
    pub fn new() -> Self {
        Self { state: Vec::new() }
    }

    pub fn push(&mut self, value: (Span, Detections)) {
        self.state.push(Some(value));
    }

    pub fn find_by_span_start(&self, start: i64) -> Vec<usize> {
        self.state
            .iter()
            .enumerate()
            .filter(|(_, value)| value.as_ref().is_some_and(|(span, _)| span.0 == start))
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    }

    pub fn take(&mut self, indices: Vec<usize>) -> Vec<(Span, Detections)> {
        indices
            .into_iter()
            .filter_map(|i| self.state.get_mut(i).unwrap().take())
            .collect()
    }
}

impl std::ops::Deref for Tracker {
    type Target = [Option<(Span, Detections)>];

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::TokenClassificationResult, pb::caikit_data_model::nlp::Token};

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
            let span = (chunk_token.start, chunk_token.end);

            let (detector_tx1, detector_rx1) = mpsc::channel(1);
            let detection = get_detection_obj(span, text, "has_HAP", "HAP");
            let _ = detector_tx1.send((chunk.clone(), vec![detection])).await;
            detection_streams.push(("hap-1".into(), detector_rx1));

            let (detector_tx2, detector_rx2) = mpsc::channel(1);
            let detection = get_detection_obj(span, text, "email_ID", "PII");
            let _ = detector_tx2.send((chunk.clone(), vec![detection])).await;
            detection_streams.push(("pii-1".into(), detector_rx2));
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
            assert_eq!(detection[0].entity_group, "HAP");
            assert_eq!(detection[1].entity_group, "PII");
            chunk_count += 1;
        }
        assert_eq!(chunk_count, chunks.len());
    }
}
