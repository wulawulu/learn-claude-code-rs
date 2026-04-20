use std::borrow::Cow;
use std::collections::HashMap;

use crate::ToolSpec;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

mod bash;
mod edit_file;
mod read_file;
mod todo;
mod write_file;
use bash::bash_tool;
use edit_file::edit_file_tool;
use read_file::read_file_tool;
use todo::todo_tool;
use write_file::write_file_tool;

pub fn toolset() -> HashMap<String, Box<dyn Tool>> {
    HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("write_file".to_string(), write_file_tool()),
        ("edit_file".to_string(), edit_file_tool()),
        ("todo".to_string(), todo_tool()),
    ])
}

#[async_trait]
pub trait Tool {
    async fn invoke(&mut self, input: &Value) -> Result<String>;
    fn name(&self) -> Cow<'_, str>;
    fn tool_spec(&self) -> ToolSpec;
}

fn safe_path(path: &str) -> Result<std::path::PathBuf> {
    let cwd = std::env::current_dir()?;
    let full = cwd.join(path).canonicalize()?;

    if !full.starts_with(&cwd) {
        return Err(anyhow::anyhow!("Path escapes workspace"));
    }

    Ok(full)
}
