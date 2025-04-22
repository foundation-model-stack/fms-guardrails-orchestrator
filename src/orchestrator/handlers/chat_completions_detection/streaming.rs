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
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{Instrument, info};

use super::ChatCompletionsDetectionTask;
use crate::{
    clients::openai::*,
    orchestrator::{Context, Error},
};

pub async fn handle_streaming(
    _ctx: Arc<Context>,
    task: ChatCompletionsDetectionTask,
) -> Result<ChatCompletionsResponse, Error> {
    let trace_id = task.trace_id;
    let detectors = task.request.detectors.clone();
    info!(%trace_id, config = ?detectors, "task started");
    let _input_detectors = detectors.input;
    let _output_detectors = detectors.output;

    // Create response channel
    let (response_tx, response_rx) =
        mpsc::channel::<Result<Option<ChatCompletionChunk>, Error>>(128);

    tokio::spawn(
        async move {
            // TODO
            let _ = response_tx
                .send(Err(Error::Validation(
                    "streaming is not yet supported".into(),
                )))
                .await;
        }
        .in_current_span(),
    );

    Ok(ChatCompletionsResponse::Streaming(response_rx))
}
