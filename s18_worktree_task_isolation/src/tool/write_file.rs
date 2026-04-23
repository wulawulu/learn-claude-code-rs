use std::borrow::Cow;
use std::path::PathBuf;

use crate::{
    ToolSpec,
    tool::{Tool, safe_path_allow_missing},
};
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::fs;

pub struct WriteFileTool {
    work_dir: PathBuf,
}

pub fn write_file_tool(work_dir: PathBuf) -> Box<dyn Tool> {
    Box::new(WriteFileTool { work_dir }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for WriteFileTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .context("Invalid path")?;
        let path = safe_path_allow_missing(&self.work_dir, path)?;

        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .context("Invalid content")?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.ok();
        }

        fs::write(&path, content)
            .await
            .map_err(|e| anyhow::anyhow!("Error: {}", e))?;

        Ok(format!(
            "Wrote {} bytes to {}",
            content.len(),
            path.display()
        ))
    }

    fn name(&self) -> Cow<'_, str> {
        "write_file".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".to_string(),
            description: Some("Write content inside the current workspace scope.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        }
    }
}
