pub mod compact;
pub mod tool;

pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;

use std::collections::HashMap;

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{ContentBlock, Message, MessageContent, MessageError, Role},
};
use anyhow::Context;

use crate::{
    compact::{CompactState, persist_large_output},
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
    pub compact_state: CompactState,
}

impl LoopState {
    pub fn new(client: AnthropicClient, tools: HashMap<String, Box<dyn Tool>>) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
            compact_state: CompactState::default(),
        }
    }

    pub async fn execute_tool_call(&mut self, content: &[ContentBlock]) -> anyhow::Result<()> {
        let mut result = Vec::new();
        let mut manual_compact = false;
        let mut compact_focus = None;
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
                        let output = if name == "bash" {
                            match persist_large_output(id, &output) {
                                Ok(compacted) => compacted,
                                Err(e) => format!("Error persisting large output: {}", e),
                            }
                        } else {
                            output
                        };

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
                        if name == "read_file"
                            && let Some(path) = input.get("path").and_then(|v| v.as_str())
                        {
                            self.remember_recent_file(path);
                        }
                        if name == "compact" {
                            println!("[manual compact]");
                            manual_compact = true;
                            compact_focus = input.get("focus").and_then(|v| v.as_str());
                        }
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
        self.context.push(Message::new_blocks(Role::User, result));
        if manual_compact {
            self.compact_history(compact_focus)
                .await
                .context("manual compact failed")?;
        }
        Ok(())
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
