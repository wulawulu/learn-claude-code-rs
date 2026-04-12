pub mod tool;

pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;

use std::collections::HashMap;

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{ContentBlock, Message, MessageContent, MessageError},
};
use anyhow::Context;

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
}

impl LoopState {
    pub fn new(client: AnthropicClient, tools: HashMap<String, Box<dyn Tool>>) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
        }
    }

    pub async fn execute_tool_call(&mut self, content: &[ContentBlock]) -> Vec<ContentBlock> {
        let mut result = Vec::new();
        for block in content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                let Some(tool) = self.tools.get_mut(name) else {
                    result.push(ContentBlock::ToolResult {
                        tool_use_id: id.clone(),
                        content: format!("Unknown tool: {}", name),
                    });
                    continue;
                };

                match tool.invoke(input).await {
                    Ok(output) => {
                        println!(
                            "Command:{}\n arg:{}\n output:\n{}\n",
                            name,
                            input,
                            output.chars().take(200).collect::<String>()
                        );
                        result.push(ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: output,
                        });
                    }
                    Err(e) => {
                        println!("Error invoking tool {}: {}", name, e);
                        result.push(ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: format!("Error invoking tool {}: {}", name, e),
                        });
                    }
                }
            }
        }
        result
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
