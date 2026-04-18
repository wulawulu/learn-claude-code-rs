use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, background::SharedBackgroundManager, tool::Tool};

pub struct BackgroundRunTool {
    manager: SharedBackgroundManager,
}

pub fn background_run_tool(manager: SharedBackgroundManager) -> Box<dyn Tool> {
    Box::new(BackgroundRunTool { manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for BackgroundRunTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let command = input
            .get("command")
            .and_then(|value| value.as_str())
            .context("Invalid command")?;
        self.manager.run(command.to_string())
    }

    fn name(&self) -> Cow<'_, str> {
        "background_run".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "background_run".to_string(),
            description: Some(
                "Run a shell command in the background and return a task id immediately."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            }),
        }
    }
}
