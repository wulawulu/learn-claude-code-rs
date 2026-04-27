use std::str::FromStr;

use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    task::{TaskStatus, TaskUpdate, render_task_json, render_task_list},
    tool::ToolContext,
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskCreateInput {
    #[schemars(description = "Short subject for the task.")]
    pub subject: String,
    #[schemars(description = "Optional detailed task description.")]
    pub description: Option<String>,
}

#[tool(name = "task_create", description = "Create a new persistent task.")]
pub async fn task_create(ctx: ToolContext, input: TaskCreateInput) -> Result<String> {
    let task = ctx.task_manager.create(
        input.subject,
        input
            .description
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    )?;
    render_task_json(&task)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskGetInput {
    #[schemars(description = "Task id to fetch.")]
    pub task_id: u64,
}

#[tool(name = "task_get", description = "Get full details of a task by ID.")]
pub async fn task_get(ctx: ToolContext, input: TaskGetInput) -> Result<String> {
    let task = ctx.task_manager.get(input.task_id)?;
    render_task_json(&task)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskListInput {}

#[tool(
    name = "task_list",
    description = "List all tasks with status summary."
)]
pub async fn task_list(ctx: ToolContext, _input: TaskListInput) -> Result<String> {
    Ok(render_task_list(ctx.task_manager.list()?))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskUpdateInput {
    #[schemars(description = "Task id to update.")]
    pub task_id: u64,
    #[schemars(description = "Optional status: pending, in_progress, completed, or deleted.")]
    pub status: Option<String>,
    #[schemars(description = "Optional owner or teammate name.")]
    pub owner: Option<String>,
    #[serde(rename = "addBlockedBy", default)]
    #[schemars(description = "Task ids that block this task.")]
    pub add_blocked_by: Vec<u64>,
    #[serde(rename = "addBlocks", default)]
    #[schemars(description = "Task ids blocked by this task.")]
    pub add_blocks: Vec<u64>,
}

#[tool(
    name = "task_update",
    description = "Update a task's status, owner, or dependencies."
)]
pub async fn task_update(ctx: ToolContext, input: TaskUpdateInput) -> Result<String> {
    let status = input
        .status
        .as_deref()
        .map(TaskStatus::from_str)
        .transpose()
        .map_err(|_| {
            anyhow::anyhow!("Invalid status. Use pending, in_progress, completed, or deleted")
        })?;

    let task = ctx.task_manager.update(
        input.task_id,
        TaskUpdate {
            status,
            owner: input.owner,
            add_blocked_by: input.add_blocked_by,
            add_blocks: input.add_blocks,
        },
    )?;
    render_task_json(&task)
}
