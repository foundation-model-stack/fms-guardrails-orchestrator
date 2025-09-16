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
use std::{
    collections::{HashMap, hash_map},
    sync::Arc,
};

use tracing::error;

use crate::{
    clients::chunker::DEFAULT_CHUNKER_ID,
    config::{DetectorConfig, DetectorType},
    models::DetectorParams,
    orchestrator::{Context, Error, types::Detection},
};

/// Slices chars between start and end indices.
pub fn slice_codepoints(text: &str, start: usize, end: usize) -> String {
    let len = end - start;
    text.chars().skip(start).take(len).collect()
}

/// Applies masks to input text, returning (offset, masked_text) pairs.
pub fn apply_masks(text: String, masks: Option<&[(usize, usize)]>) -> Vec<(usize, String)> {
    match masks {
        None | Some([]) => vec![(0, text)],
        Some(masks) => masks
            .iter()
            .map(|(start, end)| {
                let masked_text = slice_codepoints(&text, *start, *end);
                (*start, masked_text)
            })
            .collect(),
    }
}

/// Looks up chunker ids for detectors.
pub fn get_chunker_ids(
    ctx: &Arc<Context>,
    detectors: &HashMap<String, DetectorParams>,
) -> Result<Vec<String>, Error> {
    detectors
        .keys()
        .map(|detector_id| {
            let chunker_id = ctx
                .config
                .get_chunker_id(detector_id)
                .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?;
            Ok::<String, Error>(chunker_id)
        })
        .collect::<Result<Vec<_>, Error>>()
}

/// Returns the current unix timestamp.
pub fn current_timestamp() -> std::time::Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
}

/// Updates an orchestrator config, adding entries for mock servers.
/// TODO: move this to the test crate, once created.
#[cfg(test)]
pub fn configure_mock_servers(
    config: &mut crate::config::OrchestratorConfig,
    generation_server: Option<&mocktail::server::MockServer>,
    openai_server: Option<&mocktail::server::MockServer>,
    detector_servers: Option<Vec<&mocktail::server::MockServer>>,
    chunker_servers: Option<Vec<&mocktail::server::MockServer>>,
) {
    if let Some(server) = generation_server {
        let mut generation_config = crate::config::GenerationConfig::default();
        generation_config.service.hostname = "localhost".into();
        generation_config.service.port = Some(server.addr().unwrap().port());
        config.generation = Some(generation_config);
    }
    if let Some(server) = openai_server {
        let mut openai_config = crate::config::OpenAiConfig::default();
        openai_config.service.hostname = "localhost".into();
        openai_config.service.port = Some(server.addr().unwrap().port());
        config.openai = Some(openai_config);
    };
    if let Some(servers) = detector_servers {
        for server in servers {
            let mut detector_config = crate::config::DetectorConfig::default();
            detector_config.service.hostname = "localhost".into();
            detector_config.service.port = Some(server.addr().unwrap().port());
            config
                .detectors
                .insert(server.name().to_string(), detector_config);
        }
    };
    if let Some(servers) = chunker_servers {
        config.chunkers = Some(HashMap::new());
        for server in servers {
            let mut chunker_config = crate::config::ChunkerConfig::default();
            chunker_config.service.hostname = "localhost".into();
            chunker_config.service.port = Some(server.addr().unwrap().port());
            config
                .chunkers
                .as_mut()
                .unwrap()
                .insert(server.name().to_string(), chunker_config);
        }
    };
}

/// Validates requested detectors.
pub fn validate_detectors<'a>(
    detectors: impl IntoIterator<Item = (&'a String, &'a DetectorParams)>,
    orchestrator_detectors: &HashMap<String, DetectorConfig>,
    supported_detector_types: &[DetectorType],
    supports_whole_doc_chunker: bool,
) -> Result<(), Error> {
    let whole_doc_chunker_id = DEFAULT_CHUNKER_ID;
    for (detector_id, _params) in detectors {
        match orchestrator_detectors.get(detector_id) {
            Some(detector_config) => {
                if !supported_detector_types.contains(&detector_config.r#type) {
                    let error = Error::Validation(format!(
                        "detector `{detector_id}` is not supported by this endpoint"
                    ));
                    error!("{error}");
                    return Err(error);
                }
                if !supports_whole_doc_chunker && detector_config.chunker_id == whole_doc_chunker_id
                {
                    let error = Error::Validation(format!(
                        "detector `{detector_id}` uses chunker `whole_doc_chunker`, which is not supported by this endpoint"
                    ));
                    error!("{error}");
                    return Err(error);
                }
            }
            None => {
                let error = Error::DetectorNotFound(detector_id.clone());
                error!("{error}");
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Groups detectors by detector type.
pub fn group_detectors_by_type(
    ctx: &Arc<Context>,
    detectors: HashMap<String, DetectorParams>,
) -> HashMap<DetectorType, HashMap<String, DetectorParams>> {
    let mut detector_groups: HashMap<DetectorType, HashMap<String, DetectorParams>> =
        HashMap::new();
    for (detector_id, detector_params) in detectors {
        let detector_type = ctx.config.detector(&detector_id).unwrap().r#type;
        match detector_groups.entry(detector_type) {
            hash_map::Entry::Occupied(mut entry) => {
                entry.get_mut().insert(detector_id, detector_params);
            }
            hash_map::Entry::Vacant(entry) => {
                entry.insert(HashMap::from([(detector_id, detector_params)]));
            }
        }
    }
    detector_groups
}

/// Groups detections by choice.
pub fn group_detections_by_choice(
    detections: Vec<(u32, DetectorType, Vec<Detection>)>,
) -> HashMap<u32, Vec<Detection>> {
    let mut detections_by_choice: HashMap<u32, Vec<Detection>> = HashMap::new();
    for (choice_index, _, detections) in detections {
        if !detections.is_empty() {
            match detections_by_choice.entry(choice_index) {
                hash_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().extend_from_slice(&detections);
                }
                hash_map::Entry::Vacant(entry) => {
                    entry.insert(detections);
                }
            }
        }
    }
    detections_by_choice
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OrchestratorConfig;

    #[test]
    fn test_apply_masks() {
        let text = "I want this sentence. I don't want this sentence. I want this sentence too.";
        let masks: Vec<(usize, usize)> = vec![(0, 21), (50, 75)];
        let text_with_offsets = apply_masks(text.into(), Some(&masks));
        let expected_text_with_offsets = vec![
            (0, "I want this sentence.".to_string()),
            (50, "I want this sentence too.".to_string()),
        ];
        assert_eq!(text_with_offsets, expected_text_with_offsets)
    }

    #[test]
    fn test_slice_codepoints() {
        let s = "Hello world";
        assert_eq!(slice_codepoints(s, 0, 5), "Hello");
        let s = "哈囉世界";
        assert_eq!(slice_codepoints(s, 3, 4), "界");
    }

    #[test]
    fn test_validate_detectors() -> Result<(), Error> {
        let orchestrator_detectors = HashMap::from([
            (
                "pii".to_string(),
                DetectorConfig {
                    chunker_id: "sentence".into(),
                    r#type: DetectorType::TextContents,
                    ..Default::default()
                },
            ),
            (
                "pii_whole_doc".to_string(),
                DetectorConfig {
                    chunker_id: "whole_doc_chunker".into(),
                    r#type: DetectorType::TextContents,
                    ..Default::default()
                },
            ),
            (
                "granite_guardian_text_chat".to_string(),
                DetectorConfig {
                    chunker_id: "sentence".into(),
                    r#type: DetectorType::TextChat,
                    ..Default::default()
                },
            ),
        ]);

        assert!(
            validate_detectors(
                &HashMap::from([("pii".to_string(), DetectorParams::default())]),
                &orchestrator_detectors,
                &[DetectorType::TextContents],
                true
            )
            .is_ok(),
            "should pass: model is text_contents, endpoint supports text_contents"
        );
        assert!(
            validate_detectors(
                &HashMap::from([(
                    "granite_guardian_text_chat".to_string(),
                    DetectorParams::default()
                )]),
                &orchestrator_detectors,
                &[DetectorType::TextContents, DetectorType::TextChat],
                true
            )
            .is_ok(),
            "should pass: model is text_chat, endpoint supports text_contents and text_chat"
        );
        assert!(
            validate_detectors(
                &HashMap::from([("pii".to_string(), DetectorParams::default())]),
                &orchestrator_detectors,
                &[DetectorType::TextGeneration],
                true
            )
            .is_err_and(|e| matches!(e, Error::Validation(_))),
            "should fail: model is text_contents, endpoint supports text_generation"
        );
        assert!(
            validate_detectors(
                &HashMap::from([("pii_whole_doc".to_string(), DetectorParams::default())]),
                &orchestrator_detectors,
                &[DetectorType::TextContents],
                false
            )
            .is_err_and(|e| matches!(e, Error::Validation(_))),
            "should fail: model uses whole_doc_chunker and endpoint doesn't support it"
        );
        assert!(
            validate_detectors(
                &HashMap::from([("does_not_exist".to_string(), DetectorParams::default())]),
                &orchestrator_detectors,
                &[DetectorType::TextContents],
                true
            )
            .is_err_and(|e| matches!(e, Error::DetectorNotFound(_))),
            "should fail: requested model does not exist"
        );

        Ok(())
    }

    #[test]
    fn test_group_detectors_by_type() {
        let config = OrchestratorConfig {
            detectors: HashMap::from([
                (
                    "pii".to_string(),
                    DetectorConfig {
                        chunker_id: "sentence".into(),
                        r#type: DetectorType::TextContents,
                        ..Default::default()
                    },
                ),
                (
                    "hap".to_string(),
                    DetectorConfig {
                        chunker_id: "sentence".into(),
                        r#type: DetectorType::TextContents,
                        ..Default::default()
                    },
                ),
                (
                    "answer_relevance".to_string(),
                    DetectorConfig {
                        chunker_id: "whole_doc_chunker".into(),
                        r#type: DetectorType::TextGeneration,
                        ..Default::default()
                    },
                ),
            ]),
            ..Default::default()
        };
        let ctx = Arc::new(Context::new(config, Default::default()));
        let detectors = HashMap::from([
            ("pii".to_string(), DetectorParams::new()),
            ("hap".to_string(), DetectorParams::new()),
            ("answer_relevance".to_string(), DetectorParams::new()),
        ]);
        let detector_groups = group_detectors_by_type(&ctx, detectors);

        assert_eq!(detector_groups.len(), 2);
        let text_contents_detectors = detector_groups
            .iter()
            .find(|(detector_type, _)| matches!(detector_type, DetectorType::TextContents))
            .map(|(_, detectors)| detectors);
        assert!(
            text_contents_detectors.is_some_and(|d| d.len() == 2),
            "should contain text_contents detector group with 2 detectors"
        );
        assert!(
            text_contents_detectors.is_some_and(|d| d.contains_key("pii") && d.contains_key("hap")),
            "should contain text_contents detector group with pii and hap detectors"
        );
        let text_generation_detectors = detector_groups
            .iter()
            .find(|(detector_type, _)| matches!(detector_type, DetectorType::TextGeneration))
            .map(|(_, detectors)| detectors);
        assert!(
            text_generation_detectors.is_some_and(|d| d.len() == 1),
            "should contain text_generation detector group with 1 detector"
        );
        assert!(
            text_generation_detectors.is_some_and(|d| d.contains_key("answer_relevance")),
            "should contain text_generation detector group with answer_relevance detector"
        );
    }

    #[test]
    fn test_group_detections_by_choice() {
        let detections = vec![
            (
                0, // choice_index
                DetectorType::TextContents,
                vec![
                    Detection {
                        start: Some(37),
                        end: Some(51),
                        text: Some("(503) 272-8192".into()),
                        detector_id: Some("pii_detector_sentence".into()),
                        detection_type: "pii".into(),
                        detection: "PhoneNumber".into(),
                        score: 0.8,
                        ..Default::default()
                    },
                    Detection {
                        start: Some(55),
                        end: Some(69),
                        text: Some("(617) 985-3519".into()),
                        detector_id: Some("pii_detector_sentence".into()),
                        detection_type: "pii".into(),
                        detection: "PhoneNumber".into(),
                        score: 0.8,
                        ..Default::default()
                    },
                ],
            ),
            (
                0, // choice_index
                DetectorType::TextGeneration,
                vec![Detection {
                    detector_id: Some("answer_relevance_detector".into()),
                    detection_type: "risk".into(),
                    detection: "Yes".into(),
                    score: 0.8,
                    ..Default::default()
                }],
            ),
            (
                1, // choice_index
                DetectorType::TextContents,
                vec![Detection {
                    start: Some(27),
                    end: Some(41),
                    text: Some("(312) 269-1345".into()),
                    detector_id: Some("pii_detector_sentence".into()),
                    detection_type: "pii".into(),
                    detection: "PhoneNumber".into(),
                    score: 0.8,
                    ..Default::default()
                }],
            ),
        ];
        let grouped_detections = group_detections_by_choice(detections);

        assert_eq!(
            grouped_detections.keys().len(),
            2,
            "there should be keys for choice 0 and choice 1"
        );
        assert!(
            grouped_detections
                .get(&0)
                .is_some_and(|detections| detections.len() == 3),
            "there should be 3 detections for choice 0"
        );
        assert!(
            grouped_detections
                .get(&1)
                .is_some_and(|detections| detections.len() == 1),
            "there should be 1 detection for choice 1"
        );
    }
}
