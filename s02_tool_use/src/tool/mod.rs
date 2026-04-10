use std::borrow::Cow;

use crate::ToolSpec;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

mod bash;
mod edit_file;
mod read_file;
mod write_file;
pub use bash::{BashTool, bash_tool};
pub use edit_file::{EditFileTool, edit_file_tool};
pub use read_file::{ReadFileTool, read_file_tool};
pub use write_file::{WriteFileTool, write_file_tool};

#[async_trait]
pub trait Tool {
    async fn invoke(&self, input: &Value) -> Result<String>;
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
