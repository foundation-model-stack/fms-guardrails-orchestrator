use crate::pb::caikit_data_model::nlp::{ChunkerTokenizationStreamResult, TokenizationResults};

/// Internal representation of a single chunk.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Chunk {
    pub index: usize,
    pub offset: usize,
    pub start: usize,
    pub end: usize,
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

impl From<(usize, ChunkerTokenizationStreamResult)> for Chunks {
    fn from(value: (usize, ChunkerTokenizationStreamResult)) -> Self {
        let (index, value) = value;
        value
            .results
            .into_iter()
            .map(|token| Chunk {
                index,
                offset: 0, // TODO
                start: token.start as usize,
                end: token.end as usize,
                text: token.text,
            })
            .collect()
    }
}

impl From<(usize, TokenizationResults)> for Chunks {
    fn from(value: (usize, TokenizationResults)) -> Self {
        let (index, value) = value;
        value
            .results
            .into_iter()
            .map(|token| Chunk {
                index,
                offset: 0, // TODO
                start: token.start as usize,
                end: token.end as usize,
                text: token.text,
            })
            .collect()
    }
}
