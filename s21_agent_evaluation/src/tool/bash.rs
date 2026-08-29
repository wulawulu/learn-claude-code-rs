use std::time::Duration;

use crate::tool::ToolContext;
use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::{process::Command, time::timeout};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BashInput {
    #[schemars(description = "Shell command to run in the current workspace.")]
    pub command: String,
}

#[tool(
    name = "bash",
    description = "Run a shell command in the current workspace."
)]
pub async fn bash(ctx: ToolContext, input: BashInput) -> Result<String> {
    let command = input.command;
    let dangerous = ["rm -rf /", "sudo", "shutdown", "reboot", "> /dev/"];
    if dangerous.iter().any(|item| command.contains(item)) {
        anyhow::bail!("Error: dangerous command blocked");
    }

    let child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(ctx.work_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    match timeout(Duration::from_secs(120), child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let combined = [output.stdout, output.stderr].concat();
            let text = String::from_utf8_lossy(&combined);
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok("(no output)".to_string())
            } else {
                Ok(trimmed.chars().take(50_000).collect())
            }
        }
        Ok(Err(error)) => Err(error.into()),
        Err(_) => anyhow::bail!("Error: timeout (120s)"),
    }
}
