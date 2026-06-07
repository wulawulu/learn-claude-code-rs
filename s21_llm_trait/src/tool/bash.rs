use std::{borrow::Cow, time::Duration};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::{process::Command, time::timeout};

use crate::{ToolSpec, tool::Tool};

pub struct BashTool;

pub fn bash_tool() -> Box<dyn Tool> {
    Box::new(BashTool)
}

#[async_trait]
impl Tool for BashTool {
    async fn invoke(&self, input: &Value) -> Result<String> {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .context("Invalid command")?;
        let dangerous = ["rm -rf /", "sudo", "shutdown", "reboot", "> /dev/"];
        if dangerous.iter().any(|item| command.contains(item)) {
            return Err(anyhow::anyhow!("Error: Dangerous command blocked"));
        }

        let child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Error spawning command")?;

        match timeout(Duration::from_secs(120), child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let combined = [output.stdout, output.stderr].concat();
                let output = String::from_utf8_lossy(&combined);
                let output = output.trim();
                if output.is_empty() {
                    Ok("(no output)".to_string())
                } else {
                    Ok(output.chars().take(50_000).collect())
                }
            }
            Ok(Err(error)) => Err(anyhow::anyhow!("Error: {error}")),
            Err(_) => Err(anyhow::anyhow!("Error: Timeout (120s)")),
        }
    }

    fn name(&self) -> Cow<'_, str> {
        "bash".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "bash".to_string(),
            description: Some("Run a shell command in the current workspace.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }),
        }
    }
}
