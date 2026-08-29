use crate::tool::{ToolContext, safe_path};
use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::fs;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileInput {
    #[schemars(description = "Path to read, relative to the current workspace.")]
    pub path: String,
    #[schemars(description = "Optional maximum number of lines to return.")]
    pub limit: Option<u64>,
}

#[tool(name = "read_file", description = "Read file contents.")]
pub async fn read_file(ctx: ToolContext, input: ReadFileInput) -> Result<String> {
    let content = fs::read_to_string(safe_path(&ctx.work_dir, &input.path)?).await?;
    let mut lines = content.lines().map(str::to_string).collect::<Vec<_>>();

    if let Some(limit) = input.limit
        && (limit as usize) < lines.len()
    {
        let remaining = lines.len() - limit as usize;
        lines.truncate(limit as usize);
        lines.push(format!("... ({remaining} more lines)"));
    }

    Ok(lines.join("\n").chars().take(50_000).collect())
}
