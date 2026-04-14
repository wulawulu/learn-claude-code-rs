pub mod skill;
pub mod tool;

pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;

use std::{collections::HashMap, sync::Arc};

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{ContentBlock, Message, MessageContent, MessageError},
};
use anyhow::Context;

use crate::{skill::SkillRegistry, tool::Tool};

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
    pub skill_registry: Arc<SkillRegistry>,
}

impl LoopState {
    pub fn new(
        client: AnthropicClient,
        tools: HashMap<String, Box<dyn Tool>>,
        skill_registry: Arc<SkillRegistry>,
    ) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
            skill_registry,
        }
    }

    pub async fn execute_tool_call(&mut self, content: &[ContentBlock]) -> Vec<ContentBlock> {
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
        result
    }

    async fn execute(&mut self, name: &str, input: &serde_json::Value) -> String {
        let Some(tool) = self.tools.get_mut(name) else {
            return format!("Unknown tool: {name}");
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
