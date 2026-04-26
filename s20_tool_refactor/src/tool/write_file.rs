use crate::tool::{ToolContext, safe_path_allow_missing};
use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::fs;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteFileInput {
    #[schemars(description = "Path to the file to write, relative to the current workspace.")]
    pub path: String,
    #[schemars(description = "Complete file content to write.")]
    pub content: String,
}

#[tool(name = "write_file", description = "Write content to file.")]
pub async fn write_file(ctx: ToolContext, input: WriteFileInput) -> Result<String> {
    let path = safe_path_allow_missing(&ctx.work_dir, &input.path)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.ok();
    }

    fs::write(&path, &input.content)
        .await
        .map_err(|e| anyhow::anyhow!("Error: {}", e))?;

    Ok(format!(
        "Wrote {} bytes to {}",
        input.content.len(),
        path.display()
    ))
}
