use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::tool::ToolContext;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CronCreateInput {
    #[schemars(description = "Cron expression.")]
    pub cron: String,
    #[schemars(description = "Prompt to inject when the schedule fires.")]
    pub prompt: String,
    #[serde(default)]
    pub recurring: bool,
    #[serde(default)]
    pub durable: bool,
}

#[tool(name = "cron_create", description = "Create a scheduled prompt.")]
pub async fn cron_create(ctx: ToolContext, input: CronCreateInput) -> Result<String> {
    ctx.cron_scheduler
        .create(input.cron, input.prompt, input.recurring, input.durable)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CronDeleteInput {
    #[schemars(description = "Scheduled task id to delete.")]
    pub id: String,
}

#[tool(name = "cron_delete", description = "Delete a scheduled prompt.")]
pub async fn cron_delete(ctx: ToolContext, input: CronDeleteInput) -> Result<String> {
    ctx.cron_scheduler.delete(&input.id)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CronListInput {}

#[tool(name = "cron_list", description = "List scheduled prompts.")]
pub async fn cron_list(ctx: ToolContext, _input: CronListInput) -> Result<String> {
    ctx.cron_scheduler.list()
}
