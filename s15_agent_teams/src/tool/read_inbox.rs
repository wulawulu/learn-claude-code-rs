use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, team::SharedMessageBus, tool::Tool};

pub struct ReadInboxTool {
    bus: SharedMessageBus,
    inbox_owner: String,
}

pub fn read_inbox_tool(bus: SharedMessageBus, inbox_owner: impl Into<String>) -> Box<dyn Tool> {
    Box::new(ReadInboxTool {
        bus,
        inbox_owner: inbox_owner.into(),
    }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for ReadInboxTool {
    async fn invoke(&mut self, _input: &Value) -> Result<String> {
        let messages = self.bus.read_inbox(&self.inbox_owner)?;
        Ok(serde_json::to_string_pretty(&messages)?)
    }

    fn name(&self) -> Cow<'_, str> {
        "read_inbox".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_inbox".to_string(),
            description: Some("Read and drain this agent's inbox.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }
}
