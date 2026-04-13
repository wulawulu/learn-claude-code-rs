pub mod permission;
pub mod tool;
pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;
use inquire::Select;
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
    permission::{
        PermissionBehavior::{Allow, Ask, Deny},
        PermissionDecision, PermissionManager, PermissionMode,
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
    pub permission_manager: PermissionManager,
}

impl LoopState {
    pub fn new(
        client: AnthropicClient,
        tools: HashMap<String, Box<dyn Tool>>,
        permission_manager: PermissionManager,
    ) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
            permission_manager,
        }
    }

    pub async fn agent_loop(&mut self) -> Result<()> {
        let system = format!(
            r#"You are a coding agent at {}. Use tools to solve tasks.
The user controls permissions. Some tool calls may be denied."#,
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
                // Check permission
                let decision = self.permission_manager.check(name, input);
                let output;
                match decision {
                    PermissionDecision {
                        behavior: Deny,
                        reason,
                    } => {
                        output = format!("Permission denied: {}", reason);
                        println!("  [DENIED] {}: {}", name, reason);
                    }
                    PermissionDecision {
                        behavior: Allow,
                        reason: _,
                    } => {
                        output = self.execute(name, input).await;
                    }
                    PermissionDecision {
                        behavior: Ask,
                        reason: _reason,
                    } => {
                        if self.permission_manager.ask_user(name, input)? {
                            output = self.execute(name, input).await;
                        } else {
                            output = format!("Permission denied by user for {name}");
                            println!("  [USER DENIED] {name}");
                        }
                    }
                }
                result.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: output,
                });
            }
        }
        self.context.push(Message::new_blocks(Role::User, result));
        Ok(())
    }

    pub fn handle_mode_command(&mut self, query: &str) -> anyhow::Result<()> {
        let parts: Vec<&str> = query.split_whitespace().collect::<Vec<_>>();

        let mode = if parts.len() == 2 {
            parts[1].parse::<PermissionMode>().with_context(|| {
                format!(
                    "unknown mode: {}. Usage: /mode <default|plan|auto>",
                    parts[1]
                )
            })?
        } else {
            Select::new(
                "Switch permission mode:",
                vec![
                    PermissionMode::Default,
                    PermissionMode::Plan,
                    PermissionMode::Auto,
                ],
            )
            .prompt()
            .context("An error happened or user cancelled the input.")?
        };

        self.permission_manager.set_mode(mode);
        println!("[Switched to {}]", self.permission_manager.mode());

        Ok(())
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
