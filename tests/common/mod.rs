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
pub mod chunker;
pub mod detectors;
pub mod errors;
pub mod generation;
pub mod openai;
pub mod orchestrator;

/// Converts an iterator of serializable messages into an iterator of SSE data messages.
pub fn sse(
    messages: impl IntoIterator<Item = impl serde::Serialize>,
) -> impl IntoIterator<Item = String> {
    messages.into_iter().map(|msg| {
        let msg = serde_json::to_string(&msg).unwrap();
        format!("data: {msg}\n\n")
    })
}
