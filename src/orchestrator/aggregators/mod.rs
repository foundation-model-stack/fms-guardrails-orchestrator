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

mod max_processed_index;

use async_trait::async_trait;
pub use max_processed_index::MaxProcessedIndexAggregator;
use tokio::sync::{broadcast, mpsc};

use super::{
    streaming::{Chunk, Detections},
    Error,
};
use crate::models::ClassifiedGeneratedTextStreamResult;

pub type DetectorId = String;

/// Aggregates results from detection streams.
#[async_trait]
pub trait DetectionAggregator {
    async fn run(
        &self,
        mut generation_rx: broadcast::Receiver<ClassifiedGeneratedTextStreamResult>,
        detection_streams: Vec<(DetectorId, mpsc::Receiver<(Chunk, Detections)>)>,
    ) -> mpsc::Receiver<Result<ClassifiedGeneratedTextStreamResult, Error>>;
}
