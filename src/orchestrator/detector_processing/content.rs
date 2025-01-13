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

use crate::clients::detector::ContentAnalysisRequest;
use crate::orchestrator::chat_completions_detection::ChatMessageInternal;
use crate::{
    clients::openai::Content, models::DetectorParams, models::ValidationError,
    orchestrator::chat_completions_detection::ChatMessagesInternal,
};

/// Function to get content analysis request from chat message by applying rules
pub fn filter_chat_message(
    messages: ChatMessagesInternal,
) -> Result<ChatMessagesInternal, ValidationError> {
    // Implement content processing logic here
    // Rules:
    // Rule 1: Select last message from the list of messages
    let message = messages.last().unwrap();

    // Rule 2: Check if the message has content or not
    if message.content.is_none() {
        return Err(ValidationError::Invalid(
            "Message at last index does not have content".into(),
        ));
    }

    // 3. Select if message is from role `user` or `assistant` otherwise return Err
    match message.role.as_str() {
        "user" | "assistant" => (),
        _ => {
            return Err(ValidationError::Invalid(
                "Message at last index is not from user or assistant".into(),
            ))
        }
    }

    let content = match message.content.clone().unwrap() {
        Content::Text(text) => Content::Text(text),
        _ => return Err(ValidationError::Invalid("Incorrect type requested".into())),
    };
    Ok(ChatMessagesInternal::from(vec![ChatMessageInternal {
        // index of last message
        message_index: messages.len() - 1,
        role: message.role.clone(),
        content: Some(content),
        refusal: message.refusal.clone(),
    }]))
}

pub async fn get_content_analysis_request(
    messages: ChatMessagesInternal,
    detector_params: DetectorParams,
) -> Result<ContentAnalysisRequest, ValidationError> {
    if messages.is_empty() {
        return Err(ValidationError::Invalid("No messages provided".into()));
    }

    if messages.len() > 1 {
        return Err(ValidationError::Invalid(
            "More than one message is not supported".into(),
        ));
    }

    let content = match messages.first().unwrap().content.as_ref().unwrap() {
        Content::Text(text) => text,
        _ => {
            return Err(ValidationError::Invalid(
                "Message does not have content".into(),
            ))
        }
    };

    Ok(ContentAnalysisRequest::new(
        vec![content.to_string()],
        detector_params,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::orchestrator::chat_completions_detection::ChatMessagesInternal;

    #[tokio::test]
    async fn test_filter_chat_message_single_messagae() {
        let message = vec![ChatMessageInternal {
            message_index: 0,
            content: Some(Content::Text("hello".to_string())),
            role: "assistant".to_string(),
            ..Default::default()
        }];

        let filtered_messages = filter_chat_message(ChatMessagesInternal::from(message.clone()));

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
                role: "assistant".to_string(),
                ..Default::default()
            },
            ChatMessageInternal {
                message_index: 1,
                content: Some(Content::Text("bot".to_string())),
                role: "assistant".to_string(),
                ..Default::default()
            },
        ];

        let filtered_messages = filter_chat_message(ChatMessagesInternal::from(message.clone()));

        // Assertions
        assert!(filtered_messages.is_ok());
        assert_eq!(filtered_messages.unwrap(), vec![message[1].clone()]);
    }

    #[tokio::test]
    async fn test_filter_chat_messages_incorrect_role() {
        let message = vec![ChatMessageInternal {
            message_index: 0,
            content: Some(Content::Text("hello".to_string())),
            role: "invalid_role".to_string(),
            ..Default::default()
        }];

        let filtered_messages = filter_chat_message(ChatMessagesInternal::from(message.clone()));

        // Assertions
        assert!(filtered_messages.is_err());
        assert_eq!(
            filtered_messages.unwrap_err().to_string(),
            "Message at last index is not from user or assistant"
        );
    }
}
