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

// NLP generation server endpoints
pub const GENERATION_NLP_STREAMING_ENDPOINT: &str =
    "/caikit.runtime.Nlp.NlpService/ServerStreamingTextGenerationTaskPredict";
pub const GENERATION_NLP_TOKENIZATION_ENDPOINT: &str =
    "/caikit.runtime.Nlp.NlpService/TokenizationTaskPredict";

pub const GENERATION_NLP_MODEL_ID_HEADER_NAME: &str = "mm-model-id";
