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

// Detector names
pub const DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC: &str = "angle_brackets_detector_whole_doc";
pub const DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE: &str = "angle_brackets_detector_sentence";
pub const DETECTOR_NAME_PARENTHESIS_SENTENCE: &str = "parenthesis_detector_sentence";
pub const ANSWER_RELEVANCE_DETECTOR: &str = "answer_relevance_detector";
pub const FACT_CHECKING_DETECTOR: &str = "fact_checking_detector";
pub const PII_DETECTOR: &str = "pii_detector";

// Detector endpoints
pub const TEXT_CONTENTS_DETECTOR_ENDPOINT: &str = "/api/v1/text/contents";
pub const DETECTION_ON_GENERATION_DETECTOR_ENDPOINT: &str = "/api/v1/text/generation";
pub const CONTEXT_DOC_DETECTOR_ENDPOINT: &str = "/api/v1/text/context/doc";
pub const CHAT_DETECTOR_ENDPOINT: &str = "/api/v1/text/chat";
