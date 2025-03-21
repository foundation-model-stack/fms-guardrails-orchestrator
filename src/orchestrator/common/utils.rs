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
use std::{collections::HashMap, sync::Arc};

use crate::{
    models::DetectorParams,
    orchestrator::{Context, Error},
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

/// Updates an orchestrator config, adding entries for mock servers.
/// TODO: move this to the test crate, once created.
#[cfg(test)]
pub fn configure_mock_servers(
    config: &mut crate::config::OrchestratorConfig,
    generation_server: Option<&mocktail::server::MockServer>,
    chat_generation_server: Option<&mocktail::server::MockServer>,
    detector_servers: Option<Vec<&mocktail::server::MockServer>>,
    chunker_servers: Option<Vec<&mocktail::server::MockServer>>,
) {
    if let Some(server) = generation_server {
        let mut generation_config = crate::config::GenerationConfig::default();
        generation_config.service.hostname = "localhost".into();
        generation_config.service.port = Some(server.addr().unwrap().port());
        config.generation = Some(generation_config);
    }
    if let Some(server) = chat_generation_server {
        let mut chat_generation_config = crate::config::ChatGenerationConfig::default();
        chat_generation_config.service.hostname = "localhost".into();
        chat_generation_config.service.port = Some(server.addr().unwrap().port());
        config.chat_generation = Some(chat_generation_config);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
