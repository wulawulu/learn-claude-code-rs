use anyhow::{Context, Result};
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
        ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
        ChatCompletionRequestSystemMessageContent, ChatCompletionRequestToolMessage,
        ChatCompletionRequestToolMessageContent, ChatCompletionRequestUserMessage,
        ChatCompletionRequestUserMessageContent, ChatCompletionTool, ChatCompletionTools,
        CreateChatCompletionRequest, FunctionCall, FunctionObject,
    },
};
use async_trait::async_trait;

use crate::{
    ChatCompletion, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ToolCall, ToolSpec,
};

#[derive(Clone)]
pub struct OpenAIChatCompletion {
    client: Client<OpenAIConfig>,
    model: String,
    max_tokens: u32,
}

impl OpenAIChatCompletion {
    pub fn new(
        api_key: impl Into<String>,
        api_base: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let config = OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(api_base);
        Self {
            client: Client::with_config(config),
            model: model.into(),
            max_tokens: 8_000,
        }
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

#[async_trait]
impl ChatCompletion for OpenAIChatCompletion {
    async fn complete(&self, request: &ChatCompletionRequest) -> Result<ChatCompletionResponse> {
        let request = self.create_request(request)?;

        let response = self.client.chat().create(request).await?;
        let message = response
            .choices
            .into_iter()
            .next()
            .context("OpenAI returned no choices")?
            .message;
        let tool_calls = message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .filter_map(|call| match call {
                ChatCompletionMessageToolCalls::Function(call) => Some(call),
                ChatCompletionMessageToolCalls::Custom(_) => None,
            })
            .map(|call| {
                Ok(ToolCall {
                    id: call.id,
                    name: call.function.name,
                    arguments: serde_json::from_str(&call.function.arguments)
                        .context("OpenAI returned invalid tool arguments")?,
                })
            })
            .collect::<Result<_>>()?;

        Ok(ChatCompletionResponse {
            content: message.content,
            tool_calls,
        })
    }
}

impl OpenAIChatCompletion {
    // max_tokens remains the most widely supported field across OpenAI-compatible APIs.
    #[allow(deprecated)]
    fn create_request(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<CreateChatCompletionRequest> {
        Ok(CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: request
                .messages
                .iter()
                .map(to_openai_message)
                .collect::<Result<_>>()?,
            max_tokens: Some(self.max_tokens),
            tools: (!request.tools.is_empty())
                .then(|| request.tools.iter().map(to_openai_tool).collect::<Vec<_>>()),
            ..Default::default()
        })
    }
}

fn to_openai_message(message: &ChatMessage) -> Result<ChatCompletionRequestMessage> {
    Ok(match message {
        ChatMessage::System(content) => {
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(content.clone()),
                name: None,
            })
        }
        ChatMessage::User(content) => {
            ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(content.clone()),
                name: None,
            })
        }
        ChatMessage::Assistant {
            content,
            tool_calls,
        } => ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
            content: content
                .clone()
                .map(ChatCompletionRequestAssistantMessageContent::Text),
            tool_calls: (!tool_calls.is_empty()).then(|| {
                tool_calls
                    .iter()
                    .map(|call| {
                        ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                            id: call.id.clone(),
                            function: FunctionCall {
                                name: call.name.clone(),
                                arguments: call.arguments.to_string(),
                            },
                        })
                    })
                    .collect()
            }),
            ..Default::default()
        }),
        ChatMessage::Tool {
            tool_call_id,
            content,
        } => ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
            content: ChatCompletionRequestToolMessageContent::Text(content.clone()),
            tool_call_id: tool_call_id.clone(),
        }),
    })
}

fn to_openai_tool(tool: &ToolSpec) -> ChatCompletionTools {
    ChatCompletionTools::Function(ChatCompletionTool {
        function: FunctionObject {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: Some(tool.input_schema.clone()),
            strict: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_assistant_tool_call() {
        let message = ChatMessage::assistant(
            None,
            vec![ToolCall {
                id: "call-1".into(),
                name: "bash".into(),
                arguments: json!({"command": "pwd"}),
            }],
        );

        let converted = to_openai_message(&message).unwrap();
        let ChatCompletionRequestMessage::Assistant(converted) = converted else {
            panic!("expected assistant message");
        };
        assert_eq!(converted.tool_calls.unwrap().len(), 1);
    }
}
