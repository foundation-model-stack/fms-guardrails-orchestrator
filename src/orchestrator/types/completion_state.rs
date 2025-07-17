use std::{collections::BTreeMap, sync::OnceLock};

use dashmap::DashMap;

use super::ChoiceIndex;
use crate::clients::openai::Usage;

/// Completion state for a streaming completions task.
#[derive(Debug, Default)]
pub struct CompletionState<T> {
    /// Completion metadata.
    pub metadata: OnceLock<CompletionMetadata>,
    /// Completion chunks received for each choice.
    pub completions: DashMap<ChoiceIndex, BTreeMap<usize, T>>,
    /// Completion usage statistics.
    pub usage: OnceLock<Usage>,
}

impl<T> CompletionState<T>
where
    T: Default,
{
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets metadata.
    pub fn set_metadata(&self, id: String, created: i64, model: String) {
        let _ = self.metadata.set(CompletionMetadata { id, created, model });
    }

    /// Sets usage.
    pub fn set_usage(&self, usage: Usage) {
        let _ = self.usage.set(usage);
    }

    /// Inserts a completion.
    pub fn insert_completion(
        &self,
        choice_index: ChoiceIndex,
        message_index: usize,
        completion: T,
    ) {
        match self.completions.entry(choice_index) {
            dashmap::Entry::Occupied(mut entry) => {
                entry.get_mut().insert(message_index, completion);
            }
            dashmap::Entry::Vacant(entry) => {
                entry.insert(BTreeMap::from([(message_index, completion)]));
            }
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.metadata.get().map(|v| v.id.as_ref())
    }

    pub fn created(&self) -> Option<i64> {
        self.metadata.get().map(|v| v.created)
    }

    pub fn model(&self) -> Option<&str> {
        self.metadata.get().map(|v| v.model.as_ref())
    }

    pub fn usage(&self) -> Option<&Usage> {
        self.usage.get()
    }
}

/// Completion metadata common to all chunks.
#[derive(Debug, Default)]
pub struct CompletionMetadata {
    /// A unique identifier for the completion.
    pub id: String,
    /// The Unix timestamp (in seconds) of when the completion was created.
    pub created: i64,
    /// The model to generate the completion.
    pub model: String,
}
