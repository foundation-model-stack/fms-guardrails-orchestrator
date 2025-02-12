use std::collections::HashSet;

use futures::StreamExt;
use tokio::sync::broadcast;

use crate::orchestrator::types::BoxStream;

/// Consumes an input stream and forwards messages to a broadcast channel.
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
