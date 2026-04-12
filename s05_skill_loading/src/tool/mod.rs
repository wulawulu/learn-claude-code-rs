use std::borrow::Cow;

use crate::ToolSpec;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

mod bash;
mod edit_file;
mod load_skill;
mod read_file;
mod write_file;
pub use bash::{BashTool, bash_tool};
pub use edit_file::{EditFileTool, edit_file_tool};
pub use load_skill::{LoadSkillTool, load_skill_tool};
pub use read_file::{ReadFileTool, read_file_tool};
pub use write_file::{WriteFileTool, write_file_tool};

#[async_trait]
pub trait Tool: Send + Sync {
    async fn invoke(&mut self, input: &Value) -> Result<String>;
    fn name(&self) -> Cow<'_, str>;
    fn tool_spec(&self) -> ToolSpec;
}

fn safe_path(path: &str) -> Result<std::path::PathBuf> {
    resolve_safe_path(path, false)
}

fn safe_path_allow_missing(path: &str) -> Result<std::path::PathBuf> {
    resolve_safe_path(path, true)
}

fn resolve_safe_path(path: &str, allow_missing: bool) -> Result<std::path::PathBuf> {
    let cwd = std::env::current_dir()?;
    let candidate = cwd.join(path);

    let full = if candidate.exists() || !allow_missing {
        candidate.canonicalize()?
    } else {
        let parent = candidate
            .parent()
            .context("Path has no parent")?
            .canonicalize()?;

        if !parent.starts_with(&cwd) {
            return Err(anyhow::anyhow!("Path escapes workspace"));
        }

        let file_name = candidate.file_name().context("Path has no file name")?;

        parent.join(file_name)
    };

    if !full.starts_with(&cwd) {
        return Err(anyhow::anyhow!("Path escapes workspace"));
    }

    Ok(full)
}
