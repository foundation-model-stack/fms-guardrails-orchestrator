use std::collections::{btree_map, BTreeMap};

use super::{Chunk, DetectionBatcher, Detections, DetectorId, InputId};

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
pub struct MaxProcessedIndexBatcher {
    n: usize,
    state: BTreeMap<Chunk, Vec<Detections>>,
}

impl MaxProcessedIndexBatcher {
    pub fn new(n: usize) -> Self {
        Self {
            n,
            state: BTreeMap::default(),
        }
    }
}

impl DetectionBatcher for MaxProcessedIndexBatcher {
    type Batch = (Chunk, Detections);

    fn push(
        &mut self,
        _input_id: InputId,
        _detector_id: DetectorId,
        chunk: Chunk,
        detections: Detections,
    ) {
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

    fn pop_batch(&mut self) -> Option<Self::Batch> {
        // Check if we have all detections for the next chunk
        if self
            .state
            .first_key_value()
            .is_some_and(|(_, detections)| detections.len() == self.n)
        {
            // We have all detections for the chunk, remove and return it.
            if let Some((chunk, detections)) = self.state.pop_first() {
                let detections = detections.into_iter().flatten().collect();
                return Some((chunk, detections));
            }
        }
        None
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::orchestrator::types::Detection;

    #[test]
    fn test_single_chunk_multiple_detectors() {
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

        // Push detections for pii detector
        batcher.push(
            input_id,
            "pii".into(),
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

        // Push detections for hap detector
        batcher.push(
            input_id,
            "hap".into(),
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
            batch.is_some_and(|(chunk, detections)| { chunk == chunk && detections.len() == 3 })
        );
    }

    #[test]
    fn test_out_of_order_chunks() {
        todo!()
    }
}
