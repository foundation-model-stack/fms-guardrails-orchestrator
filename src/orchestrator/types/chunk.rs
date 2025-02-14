use crate::pb::caikit_data_model::nlp::{ChunkerTokenizationStreamResult, TokenizationResults};

/// Internal representation of a single chunk.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Chunk {
    pub index: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Chunks(Vec<Chunk>);

impl Chunks {
    pub fn new(values: Vec<Chunk>) -> Self {
        Self(values)
    }
}

impl std::ops::Deref for Chunks {
    type Target = Vec<Chunk>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// Conversions
// Each token represents a chunk in the chunker response.

impl From<ChunkerTokenizationStreamResult> for Chunks {
    fn from(value: ChunkerTokenizationStreamResult) -> Self {
        let index = value.input_start_index as usize;
        let chunks = value
            .results
            .into_iter()
            .map(|token| Chunk {
                index, // index + token.start as usize,
                start: token.start as usize,
                end: token.end as usize,
                text: token.text,
            })
            .collect::<Vec<_>>();
        Chunks::new(chunks)
    }
}

impl From<(usize, TokenizationResults)> for Chunks {
    fn from(value: (usize, TokenizationResults)) -> Self {
        let (index, value) = value;
        let chunks = value
            .results
            .into_iter()
            .map(|token| Chunk {
                index, // index + token.start as usize,
                start: token.start as usize,
                end: token.end as usize,
                text: token.text,
            })
            .collect::<Vec<_>>();
        Chunks::new(chunks)
    }
}
