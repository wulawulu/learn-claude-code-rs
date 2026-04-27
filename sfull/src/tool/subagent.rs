use anthropic_ai_sdk::types::message::{Message, Role};
use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    Agent, AgentSystemPrompt, extract_text, get_llm_client,
    mcp::MCPToolRouter,
    permission::{PermissionManager, PermissionMode},
    tool::{ToolContext, subagent_toolset},
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubagentInput {
    #[schemars(description = "Prompt for the subagent.")]
    pub prompt: String,
    #[schemars(description = "Short description of the task.")]
    pub description: Option<String>,
}

#[tool(
    name = "task",
    description = "Spawn a subagent with fresh context. It shares the filesystem but not conversation history."
)]
pub async fn task(ctx: ToolContext, input: SubagentInput) -> Result<String> {
    println!(
        "> task - ({}): {}",
        input.description.as_deref().unwrap_or_default(),
        input.prompt
    );

    let client = get_llm_client()?;
    let system_prompt = format!(
        "You are a coding subagent at {}. Complete the given task, then summarize your findings.",
        ctx.work_dir.display()
    );
    let mut subagent = Agent::new(
        client,
        ctx,
        subagent_toolset(),
        MCPToolRouter::new(),
        PermissionManager::try_new(PermissionMode::Default)?,
        AgentSystemPrompt::Static(system_prompt),
    );
    subagent
        .runtime
        .context
        .push(Message::new_text(Role::User, input.prompt));
    subagent.agent_loop().await?;

    let summary = subagent
        .runtime
        .context
        .iter()
        .rev()
        .find(|message| matches!(message.role, Role::Assistant))
        .map(|message| extract_text(&message.content))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "(no summary)".to_string());

    Ok(summary)
}
