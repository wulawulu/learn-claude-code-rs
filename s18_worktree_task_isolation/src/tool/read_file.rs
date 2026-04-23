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

pub struct ReadFileTool {
    work_dir: PathBuf,
}

pub fn read_file_tool(work_dir: PathBuf) -> Box<dyn Tool> {
    Box::new(ReadFileTool { work_dir }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for ReadFileTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .context("Invalid path")?;
        let path = safe_path(&self.work_dir, path)?;

        let limit = input.get("limit").and_then(|v| v.as_u64());

        let content = fs::read_to_string(path)
            .await
            .map_err(|e| anyhow::anyhow!("Error: {}", e))?;

        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        if let Some(limit) = limit
            && (limit as usize) < lines.len()
        {
            let remaining = lines.len() - limit as usize;
            lines.truncate(limit as usize);
            lines.push(format!("... ({} more lines)", remaining));
        }

        Ok(lines.join("\n").chars().take(50_000).collect())
    }

    fn name(&self) -> Cow<'_, str> {
        "read_file".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".to_string(),
            description: Some("Read file contents from the current workspace scope.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["path"]
            }),
        }
    }
}
