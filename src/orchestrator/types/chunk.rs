use crate::pb::caikit_data_model::nlp::{ChunkerTokenizationStreamResult, TokenizationResults};

/// Internal representation of a single chunk.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Input message index where this chunk begins (streaming)
    pub input_start_index: usize,
    /// Input message index where this chunk ends (streaming)
    pub input_end_index: usize,
    /// Char index where this chunk begins
    pub start: usize,
    /// Char index where this chunk ends
    pub end: usize,
    /// Text
    pub text: String,
}

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
