use crate::tool::{ToolContext, safe_path};
use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::fs;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileInput {
    #[schemars(description = "Path to the file to read, relative to the current workspace.")]
    pub path: String,
    #[schemars(
        description = "Optional maximum number of lines to return from the start of the file."
    )]
    pub limit: Option<u64>,
}

#[tool(name = "read_file", description = "Read file contents.")]
pub async fn read_file(ctx: ToolContext, input: ReadFileInput) -> Result<String> {
    let path = safe_path(&ctx.work_dir, &input.path)?;

    let content = fs::read_to_string(path)
        .await
        .map_err(|e| anyhow::anyhow!("Error: {}", e))?;

    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

    if let Some(limit) = input.limit
        && (limit as usize) < lines.len()
    {
        let remaining = lines.len() - limit as usize;
        lines.truncate(limit as usize);
        lines.push(format!("... ({} more lines)", remaining));
    }

    let result = lines.join("\n");

    Ok(result.chars().take(50000).collect())
}
