use std::{pin::Pin, task::Poll};

use futures::{stream, Stream, StreamExt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::Error;
use crate::clients::openai;

pub type ChunkerId = String;
pub type DetectorId = String;
pub type Indexed<T> = (usize, T);
pub type Chunks = Vec<Chunk>;
pub type Chunked<T> = (T, Chunks);
pub type Detections = Vec<Detection>;

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;
pub type ChunkStream<T> = BoxStream<Result<Chunked<T>, Error>>;
pub type DetectionStream<T> = BoxStream<Result<(Chunked<T>, Detections), Error>>;

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Chunk {
    pub offset: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Detection {
    pub start: Option<usize>,
    pub end: Option<usize>,
    pub text: Option<String>,
    pub detection_type: String,
    pub detection: String,
    pub detector_id: Option<String>,
    pub score: f64,
    pub evidence: Vec<DetectionEvidence>,
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct DetectionEvidence {
    pub name: String,
    pub value: Option<String>,
    pub score: Option<f64>,
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct ChatMessage<'a> {
    pub index: usize,
    pub role: Option<&'a openai::Role>,
    pub text: Option<&'a str>,
}

pub trait ChatMessageIterator {
    fn messages(&self) -> impl Iterator<Item = ChatMessage>;
}

impl ChatMessageIterator for openai::ChatCompletionsRequest {
    fn messages(&self) -> impl Iterator<Item = ChatMessage> {
        self.messages.iter().enumerate().map(|(index, message)| {
            let text = if let Some(openai::Content::Text(text)) = &message.content {
                Some(text.as_str())
            } else {
                None
            };
            ChatMessage {
                index,
                role: Some(&message.role),
                text,
            }
        })
    }
}

impl ChatMessageIterator for openai::ChatCompletion {
    fn messages(&self) -> impl Iterator<Item = ChatMessage> {
        self.choices.iter().map(|choice| ChatMessage {
            index: choice.index,
            role: Some(&choice.message.role),
            text: choice.message.content.as_deref(),
        })
    }
}

impl ChatMessageIterator for openai::ChatCompletionChunk {
    fn messages(&self) -> impl Iterator<Item = ChatMessage> {
        self.choices.iter().map(|choice| ChatMessage {
            index: choice.index as usize,
            role: choice.delta.role.as_ref(),
            text: choice.delta.content.as_deref(),
        })
    }
}

pub struct DetectionTracker {}

pub struct BatchDetectionStream<T> {
    inner: ReceiverStream<Result<(T, Detections), Error>>, // TODO: update
}

impl<T: Send + 'static> BatchDetectionStream<T> {
    pub fn new(streams: Vec<DetectionStream<T>>) -> Self {
        let _n = streams.len();
        let (_batch_tx, batch_rx) = mpsc::channel(32);
        let (batcher_tx, mut batcher_rx) = mpsc::channel(32);

        // Spawn batcher task
        tokio::spawn(async move {
            // let mut tracker = DetectionTracker::new(n);
            while let Some(_result) = batcher_rx.recv().await {
                // tracker.push(value);
                // if let Some(batch) = tracker.pop() {
                //     let _ = batch_tx.send(batch).await;
                // }
                todo!()
            }
        });

        // Spawn detection consumer task
        let mut stream_set = stream::select_all(streams);
        tokio::spawn(async move {
            while let Some(result) = stream_set.next().await {
                // Send to batcher task
                let _ = batcher_tx.send(result).await;
            }
        });

        Self {
            inner: ReceiverStream::new(batch_rx),
        }
    }
}

impl<T> Stream for BatchDetectionStream<T> {
    type Item = Result<(T, Detections), Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}
