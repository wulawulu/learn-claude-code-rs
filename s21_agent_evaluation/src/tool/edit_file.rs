use crate::tool::{ToolContext, safe_path};
use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::fs;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditFileInput {
    #[schemars(description = "Path to edit, relative to the current workspace.")]
    pub path: String,
    #[schemars(description = "Exact text to replace. Only the first match is changed.")]
    pub old_text: String,
    #[schemars(description = "Replacement text.")]
    pub new_text: String,
}

#[tool(name = "edit_file", description = "Replace exact text in a file.")]
pub async fn edit_file(ctx: ToolContext, input: EditFileInput) -> Result<String> {
    let path = safe_path(&ctx.work_dir, &input.path)?;
    let content = fs::read_to_string(&path).await?;
    if !content.contains(&input.old_text) {
        anyhow::bail!("text not found in {}", path.display());
    }

    let updated = content.replacen(&input.old_text, &input.new_text, 1);
    fs::write(&path, updated).await?;
    Ok(format!("Edited {}", path.display()))
}
