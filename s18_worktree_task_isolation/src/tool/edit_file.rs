use std::borrow::Cow;
use std::path::PathBuf;

use crate::{
    ToolSpec,
    tool::{Tool, safe_path},
};
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::fs;

pub struct EditFileTool {
    work_dir: PathBuf,
}

pub fn edit_file_tool(work_dir: PathBuf) -> Box<dyn Tool> {
    Box::new(EditFileTool { work_dir }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for EditFileTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .context("Invalid path")?;
        let path = safe_path(&self.work_dir, path)?;

        let old_text = input
            .get("old_text")
            .and_then(|v| v.as_str())
            .context("Invalid old_text")?;
        let new_text = input
            .get("new_text")
            .and_then(|v| v.as_str())
            .context("Invalid new_text")?;

        let content = fs::read_to_string(&path)
            .await
            .map_err(|e| anyhow::anyhow!("Error: {}", e))?;

        if !content.contains(old_text) {
            anyhow::bail!("Error: Text not found in {}", path.display());
        }

        let updated = content.replacen(old_text, new_text, 1);
        fs::write(&path, updated)
            .await
            .map_err(|e| anyhow::anyhow!("Error: {}", e))?;

        Ok(format!("Edited {}", path.display()))
    }

    fn name(&self) -> Cow<'_, str> {
        "edit_file".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit_file".to_string(),
            description: Some("Replace exact text inside the current workspace scope.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_text": { "type": "string" },
                    "new_text": { "type": "string" }
                },
                "required": ["path", "old_text", "new_text"]
            }),
        }
    }
}
