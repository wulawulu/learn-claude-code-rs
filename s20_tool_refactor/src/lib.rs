pub mod skill;
pub mod tool;

pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient, MessageContent, MessageError,
        RequiredMessageParams, Role, StopReason,
    },
};
use anyhow::{Context, Result};

use crate::tool::{ToolContext, ToolRouter};

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

pub struct AgentRuntime {
    pub client: AnthropicClient,
    pub context: Vec<Message>,
}

pub struct Agent {
    pub runtime: AgentRuntime,
    pub tool_context: ToolContext,
    pub tools: ToolRouter,
}

impl Agent {
    pub fn new(client: AnthropicClient, tool_context: ToolContext, tools: ToolRouter) -> Self {
        Self {
            runtime: AgentRuntime {
                client,
                context: Vec::new(),
            },
            tool_context,
            tools,
        }
    }

    pub async fn agent_loop(&mut self) -> Result<()> {
        let system = format!(
            r#"You are a coding agent at {}.
Use load_skill when a task needs specialized instructions before you act.

Skills available:
    {}
"#,
            std::env::current_dir()?.display(),
            self.tool_context.skill_registry.describe_available()
        );
        loop {
            let request = CreateMessageParams::new(RequiredMessageParams {
                model: MODEL.to_string(),
                messages: self.runtime.context.clone(),
                max_tokens: 8000,
            })
            .with_system(&system)
            .with_tools(self.tools.tool_specs());

            let response = self.runtime.client.create_message(Some(&request)).await?;

            self.runtime.context.push(Message::new_blocks(
                Role::Assistant,
                response.content.clone(),
            ));

            if let Some(stop_reason) = response.stop_reason
                && !matches!(stop_reason, StopReason::ToolUse)
            {
                return Ok(());
            }

            let tool_result = self.execute_tool_call(&response.content).await;

            self.runtime
                .context
                .push(Message::new_blocks(Role::User, tool_result));
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
        match self
            .tools
            .call(&self.tool_context, name, input.clone())
            .await
        {
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

pub type LoopState = Agent;

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
