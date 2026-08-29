pub mod eval;
pub mod judge;
pub mod tool;

pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;
pub use eval::{
    AgentRun, Assertion, AssertionEvaluator, EvalCase, EvalResult, EvalRunner, Evaluator,
    ToolCallRecord, default_cases, prepare_workspace,
};
pub use judge::LlmJudgeEvaluator;

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient, MessageContent, MessageError,
        RequiredMessageParams, Role, StopReason,
    },
};
use anyhow::{Context, Result};

use crate::tool::{ToolContext, ToolRouter};

pub fn get_model() -> Result<String> {
    dotenvy::dotenv().ok();
    std::env::var("ANTHROPIC_MODEL").context("ANTHROPIC_MODEL is not set")
}

pub fn get_llm_client() -> Result<AnthropicClient> {
    dotenvy::dotenv().ok();

    let api_key = std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY is not set")?;
    let base_url = std::env::var("ANTHROPIC_BASE_URL").context("ANTHROPIC_BASE_URL is not set")?;

    AnthropicClientBuilder::new(api_key, "")
        .with_api_base_url(base_url)
        .build::<MessageError>()
        .context("can't create client")
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

    pub async fn run(&mut self, prompt: impl Into<String>) -> Result<AgentRun> {
        self.runtime
            .context
            .push(Message::new_text(Role::User, prompt.into()));

        let system = format!(
            "You are a coding agent working at {}.\n\
             Use tools to inspect and modify only this workspace.\n\
             Complete the user's task, verify your work when appropriate, then give a concise final answer.",
            self.tool_context.work_dir.display()
        );
        let mut tool_calls = Vec::new();

        loop {
            let request = CreateMessageParams::new(RequiredMessageParams {
                model: get_model()?,
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
                return Ok(AgentRun {
                    final_answer: extract_blocks_text(&response.content),
                    tool_calls,
                });
            }

            let tool_results = self
                .execute_tool_calls(&response.content, &mut tool_calls)
                .await;
            self.runtime
                .context
                .push(Message::new_blocks(Role::User, tool_results));
        }
    }

    async fn execute_tool_calls(
        &self,
        content: &[ContentBlock],
        records: &mut Vec<ToolCallRecord>,
    ) -> Vec<ContentBlock> {
        let mut results = Vec::new();

        for block in content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                let output = match self
                    .tools
                    .call(&self.tool_context, name, input.clone())
                    .await
                {
                    Ok(output) => output,
                    Err(error) => format!("Error invoking tool {name}: {error}"),
                };

                records.push(ToolCallRecord {
                    name: name.clone(),
                    input: input.clone(),
                    output: output.clone(),
                });
                results.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: output,
                });
            }
        }

        results
    }
}

pub fn extract_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { content } => content.clone(),
        MessageContent::Blocks { content } => extract_blocks_text(content),
    }
}

fn extract_blocks_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
