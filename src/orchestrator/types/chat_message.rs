use crate::clients::openai;

/// A chat message.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct ChatMessage<'a> {
    /// Message index
    /// Corresponds to choice index for chat completions.
    pub index: u32,
    /// The role of the author of this message.
    pub role: Option<&'a openai::Role>,
    /// The text contents of the message.
    pub text: Option<&'a str>,
}

/// An iterator over chat messages.
pub trait ChatMessageIterator {
    /// Returns an iterator of [`ChatMessage`]s.
    fn messages(&self) -> impl Iterator<Item = ChatMessage>;
}

impl ChatMessageIterator for openai::ChatCompletionsRequest {
    fn messages(&self) -> impl Iterator<Item = ChatMessage> {
        self.messages.iter().enumerate().map(|(index, message)| {
            let text = if let Some(openai::Content::Text(text)) = &message.content {
                Some(text.as_str())
            } else {
                None
            };
            ChatMessage {
                index: index as u32,
                role: Some(&message.role),
                text,
            }
        })
    }
}

impl ChatMessageIterator for openai::ChatCompletion {
    fn messages(&self) -> impl Iterator<Item = ChatMessage> {
        self.choices.iter().map(|choice| ChatMessage {
            index: choice.index,
            role: Some(&choice.message.role),
            text: choice.message.content.as_deref(),
        })
    }
}

impl ChatMessageIterator for openai::ChatCompletionChunk {
    fn messages(&self) -> impl Iterator<Item = ChatMessage> {
        self.choices.iter().map(|choice| ChatMessage {
            index: choice.index,
            role: choice.delta.role.as_ref(),
            text: choice.delta.content.as_deref(),
        })
    }
}
