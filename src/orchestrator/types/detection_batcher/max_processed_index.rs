use std::collections::{btree_map, BTreeMap};

use super::{Chunk, DetectionBatcher, Detections, DetectorId, InputId};

/// A batcher based on the original "max processed index" aggregator.
///
/// Each chunk is a batch, returned in-order once detections
/// from all detectors have been received for the chunk.
///
/// Assumes all detectors are using the same chunker.
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
