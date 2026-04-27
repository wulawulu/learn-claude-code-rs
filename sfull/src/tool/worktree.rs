use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::tool::ToolContext;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorktreeCreateInput {
    pub name: String,
    pub task_id: Option<u64>,
    pub base_ref: Option<String>,
}

#[tool(
    name = "worktree_create",
    description = "Create an isolated git worktree lane."
)]
pub async fn worktree_create(ctx: ToolContext, input: WorktreeCreateInput) -> Result<String> {
    ctx.worktree_manager.create(
        input.name,
        input.task_id,
        input.base_ref.unwrap_or_else(|| "HEAD".to_string()),
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorktreeListInput {}

#[tool(name = "worktree_list", description = "List tracked worktree lanes.")]
pub async fn worktree_list(ctx: ToolContext, _input: WorktreeListInput) -> Result<String> {
    ctx.worktree_manager.list()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorktreeNameInput {
    pub name: String,
}

#[tool(
    name = "worktree_status",
    description = "Show git status for a worktree lane."
)]
pub async fn worktree_status(ctx: ToolContext, input: WorktreeNameInput) -> Result<String> {
    ctx.worktree_manager.status(&input.name)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorktreeRunInput {
    pub name: String,
    pub command: String,
}

#[tool(
    name = "worktree_run",
    description = "Run one shell command inside a named worktree."
)]
pub async fn worktree_run(ctx: ToolContext, input: WorktreeRunInput) -> Result<String> {
    ctx.worktree_manager.run(&input.name, &input.command)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorktreeEventsInput {
    pub limit: Option<usize>,
}

#[tool(
    name = "worktree_events",
    description = "List recent worktree lifecycle events."
)]
pub async fn worktree_events(ctx: ToolContext, input: WorktreeEventsInput) -> Result<String> {
    ctx.worktree_manager.events(input.limit.unwrap_or(20))
}
