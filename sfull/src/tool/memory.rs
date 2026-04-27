use anyhow::{Context as _, Result};
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{memory::MemoryType, tool::ToolContext};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveMemoryInput {
    #[schemars(description = "Short identifier, e.g. prefer_tabs or db_schema.")]
    pub name: String,
    #[schemars(description = "One-line summary of what this memory captures.")]
    pub description: String,
    #[serde(rename = "type")]
    #[schemars(description = "user, feedback, project, or reference.")]
    pub memory_type: String,
    #[schemars(description = "Full memory content.")]
    pub content: String,
}

#[tool(
    name = "save_memory",
    description = "Save a persistent memory that survives across sessions."
)]
pub async fn save_memory(ctx: ToolContext, input: SaveMemoryInput) -> Result<String> {
    let memory_type = input.memory_type.parse::<MemoryType>()?;
    let mut manager = ctx
        .memory_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("memory manager lock poisoned"))?;
    manager
        .save_memory(&input.name, &input.description, memory_type, &input.content)
        .context("failed to save memory")
}
