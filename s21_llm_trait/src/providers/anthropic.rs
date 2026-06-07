use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient, MessageContent, MessageError,
        RequiredMessageParams, Role,
    },
};
use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::{
    ChatCompletion, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ToolCall, ToolSpec,
};

#[derive(Clone)]
pub struct AnthropicChatCompletion {
    client: AnthropicClient,
    model: String,
    max_tokens: u32,
}

impl AnthropicChatCompletion {
    pub fn new(
        api_key: impl Into<String>,
        api_base: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self> {
        let client =
            AnthropicClientBuilder::new(api_key.into(), AnthropicClient::DEFAULT_API_VERSION)
                .with_api_base_url(api_base)
                .build::<MessageError>()
                .context("can't create Anthropic client")?;
        Ok(Self {
            client,
            model: model.into(),
            max_tokens: 8_000,
        })
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

#[async_trait]
impl ChatCompletion for AnthropicChatCompletion {
    async fn complete(&self, request: &ChatCompletionRequest) -> Result<ChatCompletionResponse> {
        let system = request
            .messages
            .iter()
            .filter_map(|message| match message {
                ChatMessage::System(content) => Some(content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let messages = to_anthropic_messages(&request.messages);
        let mut params = CreateMessageParams::new(RequiredMessageParams {
            model: self.model.clone(),
            messages,
            max_tokens: self.max_tokens,
        });
        if !system.is_empty() {
            params = params.with_system(system);
        }
        if !request.tools.is_empty() {
            params = params.with_tools(request.tools.iter().map(to_anthropic_tool).collect());
        }

        let response = self.client.create_message(Some(&params)).await?;
        let content = response
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let tool_calls = response
            .content
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, name, input } => Some(ToolCall {
                    id,
                    name,
                    arguments: input,
                }),
                _ => None,
            })
            .collect();

        Ok(ChatCompletionResponse {
            content: (!content.is_empty()).then_some(content),
            tool_calls,
        })
    }
}

fn to_anthropic_messages(messages: &[ChatMessage]) -> Vec<Message> {
    let mut converted = Vec::<Message>::new();
    for message in messages {
        let Some((role, blocks)) = to_anthropic_message(message) else {
            continue;
        };
        if let Some(last) = converted.last_mut()
            && same_role(last.role, role)
        {
            append_blocks(last, blocks);
        } else {
            converted.push(Message::new_blocks(role, blocks));
        }
    }
    converted
}

fn to_anthropic_message(message: &ChatMessage) -> Option<(Role, Vec<ContentBlock>)> {
    match message {
        ChatMessage::System(_) => None,
        ChatMessage::User(content) => Some((
            Role::User,
            vec![ContentBlock::Text {
                text: content.clone(),
            }],
        )),
        ChatMessage::Assistant {
            content,
            tool_calls,
        } => {
            let mut blocks = Vec::new();
            if let Some(content) = content {
                blocks.push(ContentBlock::Text {
                    text: content.clone(),
                });
            }
            blocks.extend(tool_calls.iter().map(|call| ContentBlock::ToolUse {
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.arguments.clone(),
            }));
            Some((Role::Assistant, blocks))
        }
        ChatMessage::Tool {
            tool_call_id,
            content,
        } => Some((
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: tool_call_id.clone(),
                content: content.clone(),
            }],
        )),
    }
}

fn append_blocks(message: &mut Message, blocks: Vec<ContentBlock>) {
    match &mut message.content {
        MessageContent::Blocks { content } => content.extend(blocks),
        MessageContent::Text { content } => {
            let mut merged = vec![ContentBlock::Text {
                text: std::mem::take(content),
            }];
            merged.extend(blocks);
            message.content = MessageContent::Blocks { content: merged };
        }
    }
}

fn same_role(left: Role, right: Role) -> bool {
    matches!(
        (left, right),
        (Role::User, Role::User) | (Role::Assistant, Role::Assistant)
    )
}

fn to_anthropic_tool(tool: &ToolSpec) -> anthropic_ai_sdk::types::message::Tool {
    anthropic_ai_sdk::types::message::Tool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        input_schema: tool.input_schema.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_consecutive_tool_results_for_anthropic() {
        let messages = vec![
            ChatMessage::Tool {
                tool_call_id: "one".into(),
                content: "first".into(),
            },
            ChatMessage::Tool {
                tool_call_id: "two".into(),
                content: "second".into(),
            },
        ];

        let converted = to_anthropic_messages(&messages);
        assert_eq!(converted.len(), 1);
        let MessageContent::Blocks { content } = &converted[0].content else {
            panic!("expected blocks");
        };
        assert_eq!(content.len(), 2);
    }
}
