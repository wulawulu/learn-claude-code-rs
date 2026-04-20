pub mod task;
pub mod team;
pub mod tool;
pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;
use serde_json::Value;

use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
};

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient as _, MessageContent,
        MessageError, RequiredMessageParams, Role, StopReason,
    },
};
use anyhow::{Context, Result};

use crate::team::SharedTeammateManager;
use crate::tool::Tool;

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
    pub manager: SharedTeammateManager,
    pub inbox_owner: String,
    pub system_prompt: String,
    pub max_rounds: usize,
    pub stop_signal: Option<Arc<AtomicBool>>,
}

impl LoopState {
    pub fn new(
        client: AnthropicClient,
        tools: HashMap<String, Box<dyn Tool>>,
        manager: SharedTeammateManager,
        inbox_owner: impl Into<String>,
        system_prompt: impl Into<String>,
        max_rounds: usize,
    ) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
            manager,
            inbox_owner: inbox_owner.into(),
            system_prompt: system_prompt.into(),
            max_rounds,
            stop_signal: None,
        }
    }

    pub fn with_stop_signal(mut self, stop_signal: Arc<AtomicBool>) -> Self {
        self.stop_signal = Some(stop_signal);
        self
    }

    pub async fn agent_loop(&mut self) -> Result<()> {
        for _ in 0..self.max_rounds {
            if self.should_stop() {
                return Ok(());
            }

            self.inject_inbox_messages()?;

            let request = CreateMessageParams::new(RequiredMessageParams {
                model: MODEL.to_string(),
                messages: self.context.clone(),
                max_tokens: 8000,
            })
            .with_system(&self.system_prompt)
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

        Ok(())
    }

    fn should_stop(&self) -> bool {
        self.stop_signal
            .as_ref()
            .map(|signal| signal.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(false)
    }

    fn inject_inbox_messages(&mut self) -> Result<()> {
        let inbox = self.manager.read_inbox(&self.inbox_owner)?;
        if inbox.is_empty() {
            return Ok(());
        }

        let content = format!("<inbox>{}</inbox>", serde_json::to_string_pretty(&inbox)?);
        self.context.push(Message::new_text(Role::User, content));
        Ok(())
    }

    pub async fn execute_tool_call(&mut self, content: &[ContentBlock]) -> anyhow::Result<()> {
        let mut result = Vec::new();
        for block in content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                let output = self.execute(name, input).await;
                result.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: output,
                });
            }
        }
        self.context.push(Message::new_blocks(Role::User, result));
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
