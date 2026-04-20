use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, team::SharedTeammateManager, tool::Tool};

pub struct SpawnTeammateTool {
    manager: SharedTeammateManager,
}

pub fn spawn_teammate_tool(manager: SharedTeammateManager) -> Box<dyn Tool> {
    Box::new(SpawnTeammateTool { manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for SpawnTeammateTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let name = input
            .get("name")
            .and_then(|value| value.as_str())
            .context("Invalid name")?;
        let role = input
            .get("role")
            .and_then(|value| value.as_str())
            .context("Invalid role")?;
        let prompt = input
            .get("prompt")
            .and_then(|value| value.as_str())
            .context("Invalid prompt")?;

        self.manager.spawn(name, role, prompt)
    }

    fn name(&self) -> Cow<'_, str> {
        "spawn_teammate".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "spawn_teammate".to_string(),
            description: Some("Spawn a persistent teammate that runs in its own task.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "role": { "type": "string" },
                    "prompt": { "type": "string" }
                },
                "required": ["name", "role", "prompt"]
            }),
        }
    }
}
