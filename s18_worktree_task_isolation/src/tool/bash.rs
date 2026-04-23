use std::{borrow::Cow, path::Path, path::PathBuf, time::Duration};

use crate::{ToolSpec, tool::Tool};
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::{process::Command, time::timeout};

pub struct BashTool {
    work_dir: PathBuf,
    restrict_to_workspace: bool,
}

pub fn bash_tool(work_dir: PathBuf, restrict_to_workspace: bool) -> Box<dyn Tool> {
    Box::new(BashTool {
        work_dir,
        restrict_to_workspace,
    }) as Box<dyn Tool>
}

fn validate_scoped_shell_command(work_dir: &Path, command: &str) -> Result<()> {
    if command.contains("../")
        || command.contains("..\\")
        || command.trim_start().starts_with('/')
        || command.contains(" /")
        || command.contains(" ~/")
    {
        anyhow::bail!(
            "Error: Scoped subagent shell commands must stay inside {}",
            work_dir.display()
        );
    }
    Ok(())
}

#[async_trait]
impl Tool for BashTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .context("Invalid command")?;

        let dangerous = ["rm -rf /", "sudo", "shutdown", "reboot", "> /dev/"];
        if dangerous.iter().any(|item| command.contains(item)) {
            anyhow::bail!("Error: Dangerous command blocked");
        }

        if self.restrict_to_workspace {
            validate_scoped_shell_command(&self.work_dir, command)?;
        }

        let child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.work_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow::anyhow!("Error: {}", e))?;

        let output_future = child.wait_with_output();
        match timeout(Duration::from_secs(120), output_future).await {
            Ok(Ok(output)) => {
                let combined = [output.stdout, output.stderr].concat();
                let out_str = String::from_utf8_lossy(&combined);
                let trimmed = out_str.trim();

                if trimmed.is_empty() {
                    Ok("(no output)".to_string())
                } else {
                    Ok(trimmed.chars().take(50_000).collect())
                }
            }
            Ok(Err(e)) => Err(anyhow::anyhow!("Error: {}", e)),
            Err(_) => Err(anyhow::anyhow!("Error: Timeout (120s)")),
        }
    }

    fn name(&self) -> Cow<'_, str> {
        "bash".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "bash".to_string(),
            description: Some("Run a shell command in the current workspace scope.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string"
                    }
                },
                "required": ["command"]
            }),
        }
    }
}
