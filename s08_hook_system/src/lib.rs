pub mod hook;
pub mod tool;
pub mod utils;
pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;
use serde_json::Value;

use std::collections::HashMap;

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient as _, MessageContent,
        MessageError, RequiredMessageParams, Role, StopReason,
    },
};
use anyhow::{Context, Result};

use crate::{
    hook::{
        Hook, HookControl, HookTypes, PostToolUseFn, PreToolUseFn, SessionStartFn, ToolResult,
        ToolUse,
    },
    tool::Tool,
};

pub const MODEL: &str = "deepseek-chat";

pub fn get_llm_client() -> anyhow::Result<AnthropicClient> {
    dotenvy::dotenv().ok();

    let anthropic_api_key =
        std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY is not set")?;
    let anthropic_base_url =
        std::env::var("ANTHROPIC_BASE_URL").context("ANTHROPIC_BASE_URL is not set")?;
    let client = AnthropicClientBuilder::new(anthropic_api_key, "")
        .with_api_base_url(anthropic_base_url)
        .build::<MessageError>()
        .context("can't create client")?;
    Ok(client)
}

pub struct LoopState {
    pub client: AnthropicClient,
    pub context: Vec<Message>,
    pub tools: HashMap<String, Box<dyn Tool>>,
    pub hooks: Vec<Hook>,
}

impl LoopState {
    pub fn new(client: AnthropicClient, tools: HashMap<String, Box<dyn Tool>>) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
            hooks: Vec::new(),
        }
    }

    pub async fn agent_loop(&mut self) -> Result<()> {
        let system = format!(
            "You are a coding agent at {}. Use tools to solve tasks.",
            std::env::current_dir()?.display(),
        );
        loop {
            let request = CreateMessageParams::new(RequiredMessageParams {
                model: MODEL.to_string(),
                messages: self.context.clone(),
                max_tokens: 8000,
            })
            .with_system(&system)
            .with_tools(self.tools.values().map(|tool| tool.tool_spec()).collect());

            let response = self.client.create_message(Some(&request)).await?;

            self.context.push(Message::new_blocks(
                Role::Assistant,
                response.content.clone(),
            ));

            if let Some(stop_reason) = response.stop_reason
                && !matches!(stop_reason, StopReason::ToolUse)
            {
                return Ok(());
            }

            self.execute_tool_call(&response.content).await?;
        }
    }

    pub async fn execute_tool_call(&mut self, content: &[ContentBlock]) -> anyhow::Result<()> {
        let mut result = Vec::new();
        for block in content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                let mut tool_use = ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                };

                if let HookControl::Block(reason) = invoke_hooks!(PreToolUse, self, &mut tool_use)?
                {
                    result.push(ContentBlock::ToolResult {
                        tool_use_id: tool_use.id.clone(),
                        content: format!("Tool blocked by PreToolUse hook: {reason}"),
                    });
                    continue;
                }

                let output = self.execute(&tool_use.name, &tool_use.input).await;
                let mut tool_result = ToolResult {
                    tool_use_id: tool_use.id.clone(),
                    content: output,
                };

                if let hook::HookControl::Block(reason) =
                    invoke_hooks!(PostToolUse, self, &tool_use, &mut tool_result)?
                {
                    tool_result.content = format!("Tool blocked by PostToolUse hook: {reason}");
                }

                result.push(ContentBlock::ToolResult {
                    tool_use_id: tool_result.tool_use_id,
                    content: tool_result.content,
                });
            }
        }
        self.context.push(Message::new_blocks(Role::User, result));
        Ok(())
    }

    pub fn session_start(&mut self, hook: impl SessionStartFn + 'static) {
        self.hooks.push(Hook::SessionStart(Box::new(hook)));
    }

    pub fn post_tool(&mut self, hook: impl PostToolUseFn + 'static) {
        self.hooks.push(Hook::PostToolUse(Box::new(hook)));
    }

    pub fn pre_tool(&mut self, hook: impl PreToolUseFn + 'static) {
        self.hooks.push(Hook::PreToolUse(Box::new(hook)));
    }

    pub fn hooks_by_type(&self, hook_type: HookTypes) -> Vec<&Hook> {
        self.hooks
            .iter()
            .filter(|hook| hook_type == (*hook).into())
            .collect()
    }

    async fn execute(&mut self, name: &str, input: &Value) -> String {
        let Some(tool) = self.tools.get_mut(name) else {
            return format!("Unknown tool: {}", name);
        };

        match tool.invoke(input).await {
            Ok(output) => {
                println!(
                    "Command:{}\n arg:{}\n output:\n{}\n",
                    name,
                    input,
                    output.chars().take(200).collect::<String>()
                );
                output
            }
            Err(e) => {
                println!("Error invoking tool {}: {}", name, e);
                format!("Error invoking tool {}: {}", name, e)
            }
        }
    }
}

pub fn extract_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { content } => content.clone(),
        MessageContent::Blocks { content } => content
            .iter()
            .filter_map(|block| {
                if let ContentBlock::Text { text } = block {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::{hook::HookControl, tool::Tool};

    struct DummyTool {
        output: String,
    }

    #[async_trait]
    impl Tool for DummyTool {
        async fn invoke(&mut self, _input: &Value) -> Result<String> {
            Ok(self.output.clone())
        }

        fn name(&self) -> Cow<'_, str> {
            "dummy".into()
        }

        fn tool_spec(&self) -> ToolSpec {
            ToolSpec {
                name: "dummy".to_string(),
                description: Some("dummy".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                }),
            }
        }
    }

    fn new_state() -> LoopState {
        let tools = HashMap::from([(
            "dummy".to_string(),
            Box::new(DummyTool {
                output: "tool output".to_string(),
            }) as Box<dyn Tool>,
        )]);

        let anthropic_api_key = "test".to_string();
        let anthropic_base_url = "http://localhost".to_string();
        let client = AnthropicClientBuilder::new(anthropic_api_key, "")
            .with_api_base_url(anthropic_base_url)
            .build::<MessageError>()
            .unwrap();

        LoopState::new(client, tools)
    }

    #[tokio::test]
    async fn before_tool_block_skips_execution() {
        let mut state = new_state();
        state.pre_tool(|_, _| Box::pin(async { Ok(HookControl::Block("blocked".into())) }));

        state
            .execute_tool_call(&[ContentBlock::ToolUse {
                id: "1".into(),
                name: "dummy".into(),
                input: json!({}),
            }])
            .await
            .unwrap();

        let blocks = match &state.context.last().unwrap().content {
            MessageContent::Blocks { content } => content,
            _ => panic!("expected blocks"),
        };
        assert!(
            matches!(&blocks[0], ContentBlock::ToolResult { content, .. } if content == "Tool blocked by PreToolUse hook: blocked")
        );
    }

    #[tokio::test]
    async fn post_tool_can_mutate_result() {
        let mut state = new_state();
        state.post_tool(|_, _, tool_result| {
            tool_result.content.push_str(" + modified");
            Box::pin(async { Ok(HookControl::Continue) })
        });

        state
            .execute_tool_call(&[ContentBlock::ToolUse {
                id: "1".into(),
                name: "dummy".into(),
                input: json!({}),
            }])
            .await
            .unwrap();

        let blocks = match &state.context.last().unwrap().content {
            MessageContent::Blocks { content } => content,
            _ => panic!("expected blocks"),
        };
        assert!(
            matches!(&blocks[0], ContentBlock::ToolResult { content, .. } if content == "tool output + modified")
        );
    }

    #[tokio::test]
    async fn post_tool_block_replaces_result() {
        let mut state = new_state();
        state.post_tool(|_, _, _| Box::pin(async { Ok(HookControl::Block("filtered".into())) }));

        state
            .execute_tool_call(&[ContentBlock::ToolUse {
                id: "1".into(),
                name: "dummy".into(),
                input: json!({}),
            }])
            .await
            .unwrap();

        let blocks = match &state.context.last().unwrap().content {
            MessageContent::Blocks { content } => content,
            _ => panic!("expected blocks"),
        };
        assert!(
            matches!(&blocks[0], ContentBlock::ToolResult { content, .. } if content == "Tool blocked by PostToolUse hook: filtered")
        );
    }

    #[tokio::test]
    async fn pre_tool_can_mutate_input() {
        let mut state = new_state();
        state.pre_tool(|_, tool_use| {
            tool_use.input = json!({ "rewritten": true });
            Box::pin(async { Ok(HookControl::Continue) })
        });

        state
            .execute_tool_call(&[ContentBlock::ToolUse {
                id: "1".into(),
                name: "dummy".into(),
                input: json!({}),
            }])
            .await
            .unwrap();

        let blocks = match &state.context.last().unwrap().content {
            MessageContent::Blocks { content } => content,
            _ => panic!("expected blocks"),
        };
        assert!(
            matches!(&blocks[0], ContentBlock::ToolResult { content, .. } if content == "tool output")
        );
    }

    #[tokio::test]
    async fn block_short_circuits_remaining_hooks() {
        let mut state = new_state();
        let counter = Arc::new(AtomicUsize::new(0));

        state.pre_tool(|_, _| Box::pin(async { Ok(HookControl::Block("blocked".into())) }));

        let counter_clone = counter.clone();
        state.pre_tool(move |_, _| {
            let counter = counter_clone.clone();
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(HookControl::Continue)
            })
        });

        state
            .execute_tool_call(&[ContentBlock::ToolUse {
                id: "1".into(),
                name: "dummy".into(),
                input: json!({}),
            }])
            .await
            .unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}
