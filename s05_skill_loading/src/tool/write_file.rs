use std::borrow::Cow;

use crate::{
    ToolSpec,
    tool::{Tool, safe_path_allow_missing},
};
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::fs;

pub struct WriteFileTool;

pub fn write_file_tool() -> Box<dyn Tool> {
    Box::new(WriteFileTool {}) as Box<dyn Tool>
}

#[async_trait]
impl Tool for WriteFileTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .context("Invalid path")?;
        let path = safe_path_allow_missing(path)?;

        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .context("Invalid content")?;

        // 创建父目录
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
            description: Some("Write content to file.".to_string()),
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
