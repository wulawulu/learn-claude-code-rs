use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::tool::ToolContext;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BackgroundRunInput {
    #[schemars(description = "Shell command to run in the background.")]
    pub command: String,
}

#[tool(
    name = "background_run",
    description = "Run a shell command in the background."
)]
pub async fn background_run(ctx: ToolContext, input: BackgroundRunInput) -> Result<String> {
    ctx.background_manager.run(input.command)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckBackgroundInput {
    #[schemars(description = "Optional background task id.")]
    pub task_id: Option<String>,
}

#[tool(
    name = "check_background",
    description = "Check background task status."
)]
pub async fn check_background(ctx: ToolContext, input: CheckBackgroundInput) -> Result<String> {
    ctx.background_manager.check(input.task_id.as_deref())
}
