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
use crate::{
    clients::openai::{Content, Role},
    models::ValidationError,
    orchestrator::chat_completions_detection::ChatMessageInternal,
};

/// Function to get content analysis request from chat message by applying rules
pub fn filter_chat_messages(
    messages: &[ChatMessageInternal],
) -> Result<Vec<ChatMessageInternal>, ValidationError> {
    // Get last message
    if messages.is_empty() {
        return Err(ValidationError::Invalid("No messages provided".into()));
    }
    let message = messages.last().unwrap().clone();

    // Validate message:
    // 1. Has text content
    if !matches!(message.content, Some(Content::Text(_))) {
        return Err(ValidationError::Invalid(
            "Last message content must be text".into(),
        ));
    }
    // 2. Role is user | assistant | system
    if !matches!(message.role, Role::User | Role::Assistant | Role::System) {
        return Err(ValidationError::Invalid(
            "Last message role must be user, assistant, or system".into(),
        ));
    }

    Ok(vec![ChatMessageInternal {
        message_index: message.message_index,
        role: message.role,
        content: message.content,
        refusal: message.refusal,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::chat_completions_detection::ChatMessageInternal;

    #[tokio::test]
    async fn test_filter_chat_message_single_messagae() {
        let message = vec![ChatMessageInternal {
            message_index: 0,
            content: Some(Content::Text("hello".to_string())),
            role: Role::Assistant,
            ..Default::default()
        }];

        let filtered_messages = filter_chat_messages(&message);

        // Assertions
        assert!(filtered_messages.is_ok());
        assert_eq!(filtered_messages.unwrap(), message);
    }

    #[tokio::test]
    async fn test_filter_chat_message_multiple_messages() {
        let message = vec![
            ChatMessageInternal {
                message_index: 0,
                content: Some(Content::Text("hello".to_string())),
                role: Role::Assistant,
                ..Default::default()
            },
            ChatMessageInternal {
                message_index: 1,
                content: Some(Content::Text("bot".to_string())),
                role: Role::Assistant,
                ..Default::default()
            },
        ];

        let filtered_messages = filter_chat_messages(&message);

        // Assertions
        assert!(filtered_messages.is_ok());
        assert_eq!(filtered_messages.unwrap(), vec![message[1].clone()]);
    }

    #[tokio::test]
    async fn test_filter_chat_messages_incorrect_role() {
        let message = vec![ChatMessageInternal {
            message_index: 0,
            content: Some(Content::Text("hello".to_string())),
            role: Role::Tool,
            ..Default::default()
        }];

        let filtered_messages = filter_chat_messages(&message);

        // Assertions
        assert!(filtered_messages.is_err());
        assert_eq!(
            filtered_messages.unwrap_err().to_string(),
            "Last message role must be user, assistant, or system"
        );
    }
}
