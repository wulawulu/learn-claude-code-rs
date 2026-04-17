use std::borrow::Cow;

use crate::ToolSpec;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

mod bash;
mod edit_file;
mod read_file;
mod task_create;
mod task_get;
mod task_list;
mod task_update;
mod write_file;
pub use bash::{BashTool, bash_tool};
pub use edit_file::{EditFileTool, edit_file_tool};
pub use read_file::{ReadFileTool, read_file_tool};
pub use task_create::{TaskCreateTool, task_create_tool};
pub use task_get::{TaskGetTool, task_get_tool};
pub use task_list::{TaskListTool, task_list_tool};
pub use task_update::{TaskUpdateTool, task_update_tool};
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
