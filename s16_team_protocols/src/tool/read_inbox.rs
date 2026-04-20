use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, team::SharedTeammateManager, tool::Tool};

pub struct ReadInboxTool {
    manager: SharedTeammateManager,
    inbox_owner: String,
}

pub fn read_inbox_tool(
    manager: SharedTeammateManager,
    inbox_owner: impl Into<String>,
) -> Box<dyn Tool> {
    Box::new(ReadInboxTool {
        manager,
        inbox_owner: inbox_owner.into(),
    }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for ReadInboxTool {
    async fn invoke(&mut self, _input: &Value) -> Result<String> {
        self.manager.read_inbox_json(&self.inbox_owner)
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
