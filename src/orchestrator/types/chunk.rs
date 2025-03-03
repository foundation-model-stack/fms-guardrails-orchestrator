use crate::pb::caikit_data_model::nlp::{ChunkerTokenizationStreamResult, TokenizationResults};

/// A chunk.
#[derive(Default, Debug, Clone)]
pub struct Chunk {
    /// Index of message where chunk begins (streaming only)
    pub input_start_index: usize,
    /// Index of message where chunk ends (streaming only)
    pub input_end_index: usize,
    /// Index of char where chunk begins
    pub start: usize,
    /// Index of char where chunk ends
    pub end: usize,
    /// Text
    pub text: String,
}

impl PartialOrd for Chunk {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Chunk {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.input_start_index,
            self.input_end_index,
            self.start,
            self.end,
        )
            .cmp(&(
                other.input_start_index,
                other.input_end_index,
                other.start,
                other.end,
            ))
    }
}

impl PartialEq for Chunk {
    fn eq(&self, other: &Self) -> bool {
        (
            self.input_start_index,
            self.input_end_index,
            self.start,
            self.end,
        ) == (
            other.input_start_index,
            other.input_end_index,
            other.start,
            other.end,
        )
    }
}

impl Eq for Chunk {}

impl std::hash::Hash for Chunk {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.input_start_index.hash(state);
        self.input_end_index.hash(state);
        self.start.hash(state);
        self.end.hash(state);
    }
}

/// An array of chunks.
#[derive(Default, Debug, Clone)]
pub struct Chunks(Vec<Chunk>);

impl Chunks {
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::ops::Deref for Chunks {
    type Target = Vec<Chunk>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Chunks {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoIterator for Chunks {
    type Item = Chunk;
    type IntoIter = <Vec<Chunk> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<Chunk> for Chunks {
    fn from_iter<T: IntoIterator<Item = Chunk>>(iter: T) -> Self {
        let mut chunks = Chunks::new();
        for value in iter {
            chunks.push(value);
        }
        chunks
    }
}

impl From<Vec<Chunk>> for Chunks {
    fn from(value: Vec<Chunk>) -> Self {
        Self(value)
    }
}

// Conversions

impl From<ChunkerTokenizationStreamResult> for Chunk {
    fn from(value: ChunkerTokenizationStreamResult) -> Self {
        let text = value
            .results
            .into_iter()
            .map(|token| token.text)
            .collect::<String>();
        Chunk {
            input_start_index: value.input_start_index as usize,
            input_end_index: value.input_end_index as usize,
            start: value.start_index as usize,
            end: value.processed_index as usize,
            text,
        }
    }
}

impl From<TokenizationResults> for Chunks {
    fn from(value: TokenizationResults) -> Self {
        value
            .results
            .into_iter()
            .map(|token| Chunk {
                input_start_index: 0,
                input_end_index: 0,
                start: token.start as usize,
                end: token.end as usize,
                text: token.text,
            })
            .collect()
    }
}
