use std::borrow::Cow;
use std::collections::HashMap;

use crate::ToolSpec;
use crate::task::SharedTaskManager;
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
use bash::bash_tool;
use edit_file::edit_file_tool;
use read_file::read_file_tool;
use task_create::task_create_tool;
use task_get::task_get_tool;
use task_list::task_list_tool;
use task_update::task_update_tool;
use write_file::write_file_tool;

pub fn toolset(tasks: SharedTaskManager) -> HashMap<String, Box<dyn Tool>> {
    HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("edit_file".to_string(), edit_file_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("task_create".to_string(), task_create_tool(tasks.clone())),
        ("task_get".to_string(), task_get_tool(tasks.clone())),
        ("task_list".to_string(), task_list_tool(tasks.clone())),
        ("task_update".to_string(), task_update_tool(tasks)),
        ("write_file".to_string(), write_file_tool()),
    ])
}

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
