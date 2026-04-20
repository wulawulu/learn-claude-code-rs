use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, team::SharedTeammateManager, tool::Tool};

pub struct ShutdownRequestTool {
    manager: SharedTeammateManager,
}

pub fn shutdown_request_tool(manager: SharedTeammateManager) -> Box<dyn Tool> {
    Box::new(ShutdownRequestTool { manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for ShutdownRequestTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let teammate = input
            .get("teammate")
            .and_then(|value| value.as_str())
            .context("Invalid teammate")?;
        self.manager.create_shutdown_request(teammate)
    }

    fn name(&self) -> Cow<'_, str> {
        "shutdown_request".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shutdown_request".to_string(),
            description: Some(
                "Request a teammate to shut down gracefully. Returns a request_id for tracking."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "teammate": { "type": "string" }
                },
                "required": ["teammate"]
            }),
        }
    }
}
