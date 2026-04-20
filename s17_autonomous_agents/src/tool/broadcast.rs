use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, team::SharedTeammateManager, tool::Tool};

pub struct BroadcastTool {
    manager: SharedTeammateManager,
    sender_name: String,
}

pub fn broadcast_tool(
    manager: SharedTeammateManager,
    sender_name: impl Into<String>,
) -> Box<dyn Tool> {
    Box::new(BroadcastTool {
        manager,
        sender_name: sender_name.into(),
    }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for BroadcastTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let content = input
            .get("content")
            .and_then(|value| value.as_str())
            .context("Invalid content")?;
        self.manager.broadcast(&self.sender_name, content)
    }

    fn name(&self) -> Cow<'_, str> {
        "broadcast".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "broadcast".to_string(),
            description: Some("Send a message to all teammates.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string" }
                },
                "required": ["content"]
            }),
        }
    }
}
