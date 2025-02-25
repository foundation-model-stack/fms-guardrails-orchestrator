use std::collections::{HashMap, HashSet};

use futures::StreamExt;
use tokio::sync::broadcast;

use crate::{
    config::{DetectorType, OrchestratorConfig},
    models::DetectorParams,
    orchestrator::{
        types::{BoxStream, ChunkerId, DetectorId},
        Error,
    },
};

/// Fans-out messages from an input stream to a broadcast channel.
pub fn broadcast_stream<T>(mut input_stream: BoxStream<T>) -> broadcast::Sender<T>
where
    T: Clone + Send + 'static,
{
    let (broadcast_tx, _) = broadcast::channel(32);
    tokio::spawn({
        let broadcast_tx = broadcast_tx.clone();
        async move {
            while let Some(msg) = input_stream.next().await {
                let _ = broadcast_tx.send(msg);
            }
        }
    });
    broadcast_tx
}

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

/// Filters a [`http::HeaderMap`] for specific keys.
pub fn filter_headers(keys: &HashSet<String>, headers: http::HeaderMap) -> http::HeaderMap {
    headers
        .iter()
        .filter(|(name, _)| keys.contains(&name.as_str().to_lowercase()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// Looks up chunker ids for detectors.
pub fn get_chunker_ids(
    config: &OrchestratorConfig,
    detectors: &HashMap<String, DetectorParams>,
) -> Result<Vec<ChunkerId>, Error> {
    detectors
        .keys()
        .map(|detector_id| {
            let chunker_id = config
                .get_chunker_id(detector_id)
                .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?;
            Ok::<_, Error>(chunker_id)
        })
        .collect::<Result<Vec<_>, Error>>()
}

/// Validates requested detectors exist and are supported types.
pub fn validate_detectors(
    config: &OrchestratorConfig,
    detector_ids: &[DetectorId],
    supported_types: &[DetectorType],
) -> Result<(), Error> {
    for detector_id in detector_ids {
        let config = config
            .detector(detector_id)
            .ok_or_else(|| Error::DetectorNotFound(detector_id.clone()))?;
        if !supported_types.contains(&config.r#type) {
            return Err(Error::DetectorNotSupported(detector_id.clone()));
        }
    }
    Ok(())
}

pub fn current_timestamp_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::config::DetectorConfig;

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
        let mut config = OrchestratorConfig::default();
        let d1 = (
            "d1".to_string(),
            DetectorConfig {
                r#type: DetectorType::TextContents,
                ..Default::default()
            },
        );
        let d2 = (
            "d2".to_string(),
            DetectorConfig {
                r#type: DetectorType::TextChat,
                ..Default::default()
            },
        );
        config.detectors = HashMap::from_iter([d1, d2]);

        assert_eq!(
            validate_detectors(
                &config,
                &["d1".to_string(), "d2".to_string()],
                &[DetectorType::TextContents],
            ),
            Err(Error::DetectorNotSupported("d2".to_string()))
        );

        assert!(validate_detectors(
            &config,
            &["d1".to_string(), "d2".to_string()],
            &[DetectorType::TextContents, DetectorType::TextChat],
        )
        .is_ok());

        assert_eq!(
            validate_detectors(&config, &["d3".to_string()], &[DetectorType::TextContents],),
            Err(Error::DetectorNotFound("d3".to_string()))
        );

        Ok(())
    }
}
