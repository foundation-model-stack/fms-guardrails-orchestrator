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

/// A batcher based on the original "max processed index"
/// aggregator.
///
/// A batch corresponds to a chunk. Batches are
/// returned in-order as detections from all detectors
/// are received for the chunk.
///
/// For example, if we have n chunks with 3 detectors
/// applied to each chunk, the first batch for chunk-1 is
/// popped once detections from 3 detectors are received
/// for chunk-1. The next batch for chunk-2 is popped once
/// detections from 3 detectors are received for chunk-2,
/// and so on.
///
/// This batcher requires that all detectors use the same chunker.
#[derive(Debug, Clone)]
pub struct MaxProcessedIndexBatcher {
    n_detectors: usize,
    state: BTreeMap<Chunk, Vec<Detections>>,
}

impl MaxProcessedIndexBatcher {
    pub fn new(n_detectors: usize) -> Self {
        Self {
            n_detectors,
            state: BTreeMap::default(),
        }
    }
}

impl DetectionBatcher for MaxProcessedIndexBatcher {
    fn push(&mut self, _input_id: u32, chunk: Chunk, detections: Detections) {
        match self.state.entry(chunk) {
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
        // Check if we have all detections for the next chunk
        if self
            .state
            .first_key_value()
            .is_some_and(|(_, detections)| detections.len() == self.n_detectors)
        {
            // We have all detections for the chunk, remove and return it.
            if let Some((chunk, detections)) = self.state.pop_first() {
                let detections = detections.into_iter().flatten().collect();
                return Some((0, chunk, detections));
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
        let input_id = 0;
        let chunk = Chunk {
            input_start_index: 0,
            input_end_index: 0,
            start: 0,
            end: 24,
            text: "this is a dummy sentence".into(),
        };

        // Create a batcher that will process batches for 2 detectors
        let n = 2;
        let mut batcher = MaxProcessedIndexBatcher::new(n);

        // Push chunk detections for pii detector
        batcher.push(
            input_id,
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
            input_id,
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
        assert!(batch.is_some_and(|(_input_id, chunk, detections)| {
            chunk == chunk && detections.len() == 3
        }));
    }

    #[test]
    fn test_batcher_with_out_of_order_chunks() {
        let input_id = 0;
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
        let n = 2;
        let mut batcher = MaxProcessedIndexBatcher::new(n);

        // NOTE: Both chunk-2 detections are pushed for detectors here before their
        // respective chunk-1 detections. At this batcher level, ordering will be
        // expected but may present as an edge case at the stream level ref.
        // https://github.com/foundation-model-stack/fms-guardrails-orchestrator/issues/377

        // Push chunk-2 detections for pii detector
        batcher.push(
            input_id,
            chunks[1].clone(),
            Detections::default(), // no detections
        );
        // Push chunk-2 detections for hap detector
        batcher.push(
            input_id,
            chunks[1].clone(),
            Detections::default(), // no detections
        );
        // Push chunk-1 detections for hap detector
        batcher.push(
            input_id,
            chunks[0].clone(),
            Detections::default(), // no detections
        );

        // We have all detections for chunk-2, but not chunk-1
        // pop_batch() should return None
        assert!(batcher.pop_batch().is_none());

        // Push chunk-1 detections for pii detector
        batcher.push(
            input_id,
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

        // We have all detections for chunk-1 and chunk-2
        // pop_batch() should return chunk-1 with 1 pii detection
        let batch = batcher.pop_batch();
        assert!(batch.is_some_and(|(_input_id, chunk, detections)| {
            chunk == chunks[0] && detections.len() == 1
        }));

        // pop_batch() should return chunk-2 with no detections
        let batch = batcher.pop_batch();
        assert!(batch.is_some_and(|(_input_id, chunk, detections)| {
            chunk == chunks[1] && detections.is_empty()
        }));

        // batcher state should be empty as all batches have been returned
        assert!(batcher.state.is_empty());
    }

    #[tokio::test]
    async fn test_detection_batch_stream() -> Result<(), Error> {
        let input_id = 0;
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
            mpsc::channel::<Result<(u32, Chunk, Detections), Error>>(4);
        let pii_detections_stream = ReceiverStream::new(pii_detections_rx).boxed();
        let (hap_detections_tx, hap_detections_rx) =
            mpsc::channel::<Result<(u32, Chunk, Detections), Error>>(4);
        let hap_detections_stream = ReceiverStream::new(hap_detections_rx).boxed();

        // Create a batcher that will process batches for 2 detectors
        let n = 2;
        let batcher = MaxProcessedIndexBatcher::new(n);

        // Create detection batch stream
        let streams = vec![pii_detections_stream, hap_detections_stream];
        let mut detection_batch_stream = DetectionBatchStream::new(batcher, streams);

        // Send chunk-2 detections for pii detector
        let _ = pii_detections_tx
            .send(Ok((
                input_id,
                chunks[1].clone(),
                Detections::default(), // no detections
            )))
            .await;

        // Send chunk-1 detections for hap detector
        let _ = hap_detections_tx
            .send(Ok((
                input_id,
                chunks[0].clone(),
                Detections::default(), // no detections
            )))
            .await;

        // Send chunk-2 detections for hap detector
        let _ = hap_detections_tx
            .send(Ok((
                input_id,
                chunks[1].clone(),
                Detections::default(), // no detections
            )))
            .await;

        // We have all detections for chunk-2, but not chunk-1
        // detection_batch_stream.next() future should not be ready
        assert!(matches!(
            futures::poll!(detection_batch_stream.next()),
            Poll::Pending
        ));

        // Send chunk-1 detections for pii detector
        let _ = pii_detections_tx
            .send(Ok((
                input_id,
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

        // We have all detections for chunk-1 and chunk-2
        // detection_batch_stream.next() should be ready and return chunk-1 with 1 pii detection
        let batch = detection_batch_stream.next().await;
        assert!(batch.is_some_and(|result| {
            result.is_ok_and(|(_input_id, chunk, detections)| {
                chunk == chunks[0] && detections.len() == 1
            })
        }));

        // detection_batch_stream.next() should be ready and return chunk-2 with no detections
        let batch = detection_batch_stream.next().await;
        assert!(batch.is_some_and(|result| {
            result.is_ok_and(|(_input_id, chunk, detections)| {
                chunk == chunks[1] && detections.is_empty()
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
