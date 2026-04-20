use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{
    ToolSpec,
    team::{SharedMessageBus, SharedTeammateManager},
    tool::Tool,
};

pub struct BroadcastTool {
    bus: SharedMessageBus,
    manager: SharedTeammateManager,
}

pub fn broadcast_tool(bus: SharedMessageBus, manager: SharedTeammateManager) -> Box<dyn Tool> {
    Box::new(BroadcastTool { bus, manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for BroadcastTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let content = input
            .get("content")
            .and_then(|value| value.as_str())
            .context("Invalid content")?;
        let names = self.manager.member_names()?;
        self.bus.broadcast("lead", content, &names)
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
