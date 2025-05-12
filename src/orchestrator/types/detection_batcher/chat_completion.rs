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
use std::collections::{BTreeMap, btree_map};

use super::{Batch, Chunk, DetectionBatcher, Detections};

pub type ChoiceIndex = u32;

/// A batcher for chat completions.
///
/// A batch corresponds to a choice-chunk (where each chunk is associated
/// with a particular choice through a ChoiceIndex). Batches are returned
/// in-order as detections from all detectors are received for the choice-chunk.
///
/// Chat completion messages have a `choices` field containing
/// a single choice, e.g.
/// ```text
///     data: {"id":"chat-", ..., "choices":[{"index":0, ...}]}
///     data: {"id":"chat-", ..., "choices":[{"index":1, ...}]}
/// ```
/// And we track chunks for each choice independently.
///
/// This batcher requires that all detectors use the same chunker.
#[derive(Debug, Clone)]
pub struct ChatCompletionBatcher {
    n_detectors: usize,
    // We place the chunk first since chunk ordering includes where
    // the chunk is in all the processed messages.
    state: BTreeMap<(Chunk, ChoiceIndex), Vec<Detections>>,
}

impl ChatCompletionBatcher {
    pub fn new(n_detectors: usize) -> Self {
        Self {
            n_detectors,
            state: BTreeMap::default(),
        }
    }
}

impl DetectionBatcher for ChatCompletionBatcher {
    fn push(&mut self, choice_index: ChoiceIndex, chunk: Chunk, detections: Detections) {
        match self.state.entry((chunk, choice_index)) {
            btree_map::Entry::Vacant(entry) => {
                // New chunk, insert entry
                entry.insert(vec![detections]);
            }
            btree_map::Entry::Occupied(mut entry) => {
                // Existing chunk, push detections
                entry.get_mut().push(detections);
            }
        }
    }

    fn pop_batch(&mut self) -> Option<Batch> {
        // Batching logic here will only assume detections with the same chunker type
        // Requirements in https://github.com/foundation-model-stack/fms-guardrails-orchestrator/blob/main/docs/architecture/adrs/005-chat-completion-support.md#streaming-response
        // for detections on whole output will be handled outside of the batcher

        // Check if we have all detections for the next chunk
        if self
            .state
            .first_key_value()
            .is_some_and(|(_, detections)| detections.len() == self.n_detectors)
        {
            // We have all detections for the chunk, remove and return it.
            if let Some(((chunk, choice_index), detections)) = self.state.pop_first() {
                let detections = detections.into_iter().flatten().collect();
                return Some((choice_index, chunk, detections));
            }
        }
        None
    }

    fn is_empty(&self) -> bool {
        self.state.is_empty()
    }
}

#[cfg(test)]
mod test {
    use std::task::Poll;

    use futures::StreamExt;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    use super::*;
    use crate::orchestrator::{
        Error,
        types::{Detection, DetectionBatchStream},
    };

    #[test]
    fn test_batcher_with_single_chunk() {
        let choice_index = 0;
        let chunk = Chunk {
            input_start_index: 0,
            input_end_index: 0,
            start: 0,
            end: 24,
            text: "this is a dummy sentence".into(),
        };

        // Create a batcher that will process batches for 2 detectors
        let n_detectors = 2;
        let mut batcher = ChatCompletionBatcher::new(n_detectors);

        // Push chunk detections for pii detector
        batcher.push(
            choice_index,
            chunk.clone(),
            vec![Detection {
                start: Some(5),
                end: Some(10),
                detector_id: Some("pii".into()),
                detection_type: "pii".into(),
                score: 0.4,
                ..Default::default()
            }]
            .into(),
        );

        // We only have detections for 1 detector
        // pop_batch() should return None
        assert!(batcher.pop_batch().is_none());

        // Push chunk detections for hap detector
        batcher.push(
            choice_index,
            chunk.clone(),
            vec![
                Detection {
                    start: Some(5),
                    end: Some(10),
                    detector_id: Some("hap".into()),
                    detection_type: "hap".into(),
                    score: 0.8,
                    ..Default::default()
                },
                Detection {
                    start: Some(15),
                    end: Some(20),
                    detector_id: Some("hap".into()),
                    detection_type: "hap".into(),
                    score: 0.8,
                    ..Default::default()
                },
            ]
            .into(),
        );

        // We have detections for 2 detectors
        // pop_batch() should return a batch containing 3 detections for the chunk
        let batch = batcher.pop_batch();
        assert!(
            batch.is_some_and(|(actual_choice_index, actual_chunk, detections)| {
                actual_chunk == chunk
                    && actual_choice_index == choice_index
                    && detections.len() == 3
            })
        );
    }

    #[test]
    fn test_batcher_with_out_of_order_chunks_same_per_choice() {
        let choices = 2;
        // Chunks here will be apply to both choices
        let chunks = [
            Chunk {
                input_start_index: 0,
                input_end_index: 10,
                start: 0,
                end: 56,
                text: " a powerful tool for the development \
                    of complex systems."
                    .into(),
            },
            Chunk {
                input_start_index: 11,
                input_end_index: 26,
                start: 56,
                end: 135,
                text: " It has been used in many fields, such as \
                    computer vision and image processing."
                    .into(),
            },
        ];

        // Create a batcher that will process batches for 2 detectors
        let n_detectors = 2;
        let mut batcher = ChatCompletionBatcher::new(n_detectors);

        for choice_index in 0..choices {
            // Push chunk-2 detections for pii detector
            batcher.push(
                choice_index,
                chunks[1].clone(),
                Detections::default(), // no detections
            );
            // Push chunk-1 detections for hap detector
            batcher.push(
                choice_index,
                chunks[0].clone(),
                Detections::default(), // no detections
            );
            // Push chunk-2 detections for hap detector
            batcher.push(
                choice_index,
                chunks[1].clone(),
                Detections::default(), // no detections
            );
        }

        // We have all detections for chunk-2, but not chunk-1
        // pop_batch() should return None
        assert!(batcher.pop_batch().is_none());

        // Push chunk-1 detections for pii detector
        for choice_index in 0..choices {
            batcher.push(
                choice_index,
                chunks[0].clone(),
                vec![Detection {
                    start: Some(10),
                    end: Some(20),
                    detector_id: Some("pii".into()),
                    detection_type: "pii".into(),
                    score: 0.4,
                    ..Default::default()
                }]
                .into(),
            );
        }

        // We have all detections for chunk-1 and chunk-2
        // pop_batch() should return chunk-1 with 1 pii detection, for the first choice
        let batch = batcher.pop_batch();
        assert!(batch.is_some_and(|(choice_index, chunk, detections)| {
            chunk == chunks[0] && choice_index == 0 && detections.len() == 1
        }));

        // Return the same chunk-1 with 1 pii detection for the second choice
        let batch = batcher.pop_batch();
        assert!(batch.is_some_and(|(choice_index, chunk, detections)| {
            chunk == chunks[0] && choice_index == 1 && detections.len() == 1
        }));

        // pop_batch() should return chunk-2 with no detections, for the first choice
        let batch = batcher.pop_batch();
        assert!(batch.is_some_and(|(choice_index, chunk, detections)| {
            chunk == chunks[1] && choice_index == 0 && detections.is_empty()
        }));

        // Return the same chunk-2 with no detections for the second choice
        let batch = batcher.pop_batch();
        assert!(batch.is_some_and(|(choice_index, chunk, detections)| {
            chunk == chunks[1] && choice_index == 1 && detections.is_empty()
        }));

        // batcher state should be empty as all batches have been returned
        assert!(batcher.state.is_empty());
    }

    #[test]
    fn test_batcher_with_out_of_order_chunks_different_per_choice() {
        // Chunks here will be apply to the first choice
        let choice_1_index = 0;
        let choice_1_chunks = [
            Chunk {
                input_start_index: 0,
                input_end_index: 10,
                start: 0,
                end: 46,
                text: " a tool for the development \
                    of simple systems."
                    .into(),
            },
            Chunk {
                input_start_index: 11,
                input_end_index: 26,
                start: 46,
                end: 125,
                text: " It has been used in many fields, such as \
                    computer vision and audio processing."
                    .into(),
            },
        ];

        // Chunks here will apply to the second choice
        let choice_2_index = 1;
        let choice_2_chunks = [
            Chunk {
                input_start_index: 0,
                input_end_index: 10,
                start: 0,
                end: 56,
                text: " a powerful tool for the development \
                    of complex systems."
                    .into(),
            },
            Chunk {
                input_start_index: 11,
                input_end_index: 26,
                start: 56,
                end: 135,
                text: " It has been used in many fields, such as \
                    computer vision and image processing."
                    .into(),
            },
        ];

        // Create a batcher that will process batches for 2 detectors
        let n_detectors = 2;
        let mut batcher = ChatCompletionBatcher::new(n_detectors);

        // Intersperse choice detections
        // NOTE: There may be an edge case when chunk-2 (or later) detections are pushed
        // for all detectors here before their respective earlier detections (e.g. chunk-1 here).
        // At this batcher level, ordering will be expected but may present as an edge case
        // at the stream level ref.
        // https://github.com/foundation-model-stack/fms-guardrails-orchestrator/issues/377

        // Push chunk-2 detections for pii detector, choice 1
        batcher.push(
            choice_1_index,
            choice_1_chunks[1].clone(),
            Detections::default(), // no detections
        );
        // Same for choice 2
        batcher.push(
            choice_2_index,
            choice_2_chunks[1].clone(),
            Detections::default(), // no detections
        );
        // Push chunk-2 detections for hap detector, choice 2
        batcher.push(
            choice_2_index,
            choice_2_chunks[1].clone(),
            Detections::default(), // no detections
        );
        // Same for choice 1
        batcher.push(
            choice_1_index,
            choice_1_chunks[1].clone(),
            Detections::default(), // no detections
        );
        // Push chunk-1 detections for hap detector, choice 1
        batcher.push(
            choice_1_index,
            choice_1_chunks[0].clone(),
            Detections::default(), // no detections
        );
        // Same for choice 2
        batcher.push(
            choice_2_index,
            choice_2_chunks[0].clone(),
            Detections::default(), // no detections
        );

        // We have all detections for chunk-2, but not chunk-1, for both choices
        // pop_batch() should return None
        assert!(batcher.pop_batch().is_none());

        // Push chunk-1 detections for pii detector, for first choice
        batcher.push(
            choice_1_index,
            choice_1_chunks[0].clone(),
            vec![Detection {
                start: Some(10),
                end: Some(20),
                detector_id: Some("pii".into()),
                detection_type: "pii".into(),
                score: 0.4,
                ..Default::default()
            }]
            .into(),
        );
        // Push chunk-1 detections for pii detector, for second choice
        batcher.push(
            choice_2_index,
            choice_2_chunks[0].clone(),
            vec![Detection {
                start: Some(10),
                end: Some(20),
                detector_id: Some("pii".into()),
                detection_type: "pii".into(),
                score: 0.4,
                ..Default::default()
            }]
            .into(),
        );

        // We have all detections for chunk-1 and chunk-2
        // Expect 4 chunks, with those for the chunk-1 chunks first
        let batch = batcher.pop_batch();
        assert!(batch.is_some_and(|(choice_index, chunk, detections)| {
            chunk == choice_1_chunks[0] && choice_index == choice_1_index && detections.len() == 1
        }));
        let batch = batcher.pop_batch();
        assert!(batch.is_some_and(|(choice_index, chunk, detections)| {
            chunk == choice_2_chunks[0] && choice_index == choice_2_index && detections.len() == 1
        }));

        // chunk-2 chunks
        let batch = batcher.pop_batch();
        assert!(batch.is_some_and(|(choice_index, chunk, detections)| {
            chunk == choice_1_chunks[1] && choice_index == choice_1_index && detections.is_empty()
        }));
        let batch = batcher.pop_batch();
        assert!(batch.is_some_and(|(choice_index, chunk, detections)| {
            chunk == choice_2_chunks[1] && choice_index == choice_2_index && detections.is_empty()
        }));

        // batcher state should be empty as all batches (4 chunks) have been returned
        assert!(batcher.state.is_empty());
    }

    #[tokio::test]
    async fn test_detection_batch_stream_chat() -> Result<(), Error> {
        let choices = 2;
        // Chunks here will be apply to both choices
        let chunks = [
            Chunk {
                input_start_index: 0,
                input_end_index: 10,
                start: 0,
                end: 56,
                text: " a powerful tool for the development \
                    of complex systems."
                    .into(),
            },
            Chunk {
                input_start_index: 11,
                input_end_index: 26,
                start: 56,
                end: 135,
                text: " It has been used in many fields, such as \
                    computer vision and image processing."
                    .into(),
            },
        ];

        // Create detection channels and streams
        let (pii_detections_tx, pii_detections_rx) =
            mpsc::channel::<Result<(ChoiceIndex, Chunk, Detections), Error>>(4);
        let pii_detections_stream = ReceiverStream::new(pii_detections_rx).boxed();
        let (hap_detections_tx, hap_detections_rx) =
            mpsc::channel::<Result<(ChoiceIndex, Chunk, Detections), Error>>(4);
        let hap_detections_stream = ReceiverStream::new(hap_detections_rx).boxed();

        // Create a batcher that will process batches for 2 detectors
        let n_detectors = 2;
        let batcher = ChatCompletionBatcher::new(n_detectors);

        // Create detection batch stream
        let streams = vec![pii_detections_stream, hap_detections_stream];
        let mut detection_batch_stream = DetectionBatchStream::new(batcher, streams);

        for choice_index in 0..choices {
            // Send chunk-2 detections for pii detector
            let _ = pii_detections_tx
                .send(Ok((
                    choice_index,
                    chunks[1].clone(),
                    Detections::default(), // no detections
                )))
                .await;

            // Send chunk-1 detections for hap detector
            let _ = hap_detections_tx
                .send(Ok((
                    choice_index,
                    chunks[0].clone(),
                    Detections::default(), // no detections
                )))
                .await;

            // Send chunk-2 detections for hap detector
            let _ = hap_detections_tx
                .send(Ok((
                    choice_index,
                    chunks[1].clone(),
                    Detections::default(), // no detections
                )))
                .await;
        }

        // We have all detections for chunk-2, but not chunk-1
        // detection_batch_stream.next() future should not be ready
        assert!(matches!(
            futures::poll!(detection_batch_stream.next()),
            Poll::Pending
        ));

        // Send chunk-1 detections for pii detector
        for choice_index in 0..choices {
            let _ = pii_detections_tx
                .send(Ok((
                    choice_index,
                    chunks[0].clone(),
                    vec![Detection {
                        start: Some(10),
                        end: Some(20),
                        detector_id: Some("pii".into()),
                        detection_type: "pii".into(),
                        score: 0.4,
                        ..Default::default()
                    }]
                    .into(),
                )))
                .await;
        }

        // We have all detections for chunk-1 and chunk-2
        // detection_batch_stream.next() should be ready and return chunk-1 with 1 pii detection, for choice 1
        let batch = detection_batch_stream.next().await;
        assert!(batch.is_some_and(|result| {
            result.is_ok_and(|(choice_index, chunk, detections)| {
                chunk == chunks[0] && choice_index == 0 && detections.len() == 1
            })
        }));

        // Then choice 2
        let batch = detection_batch_stream.next().await;
        assert!(batch.is_some_and(|result| {
            result.is_ok_and(|(choice_index, chunk, detections)| {
                chunk == chunks[0] && choice_index == 1 && detections.len() == 1
            })
        }));

        // detection_batch_stream.next() should be ready and return chunk-2 with no detections, for choice 1
        let batch = detection_batch_stream.next().await;
        assert!(batch.is_some_and(|result| {
            result.is_ok_and(|(choice_index, chunk, detections)| {
                chunk == chunks[1] && choice_index == 0 && detections.is_empty()
            })
        }));

        // Then choice 2
        let batch = detection_batch_stream.next().await;
        assert!(batch.is_some_and(|result| {
            result.is_ok_and(|(choice_index, chunk, detections)| {
                chunk == chunks[1] && choice_index == 1 && detections.is_empty()
            })
        }));

        // detection_batch_stream.next() future should not be ready
        // as detection senders have not been closed
        assert!(matches!(
            futures::poll!(detection_batch_stream.next()),
            Poll::Pending
        ));

        // Drop detection senders
        drop(pii_detections_tx);
        drop(hap_detections_tx);

        // detection_batch_stream.next() should return None
        assert!(detection_batch_stream.next().await.is_none());

        Ok(())
    }
}
