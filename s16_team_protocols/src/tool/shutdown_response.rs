use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, team::SharedTeammateManager, tool::Tool};

pub struct ShutdownResponseTool {
    manager: SharedTeammateManager,
    sender_name: String,
}

pub struct ShutdownResponseStatusTool {
    manager: SharedTeammateManager,
}

pub fn shutdown_response_tool(
    manager: SharedTeammateManager,
    sender_name: impl Into<String>,
) -> Box<dyn Tool> {
    Box::new(ShutdownResponseTool {
        manager,
        sender_name: sender_name.into(),
    }) as Box<dyn Tool>
}

pub fn shutdown_response_status_tool(manager: SharedTeammateManager) -> Box<dyn Tool> {
    Box::new(ShutdownResponseStatusTool { manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for ShutdownResponseTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let request_id = input
            .get("request_id")
            .and_then(|value| value.as_str())
            .context("Invalid request_id")?;
        let approve = input
            .get("approve")
            .and_then(|value| value.as_bool())
            .context("Invalid approve")?;
        let reason = input
            .get("reason")
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        let output =
            self.manager
                .respond_shutdown(&self.sender_name, request_id, approve, reason)?;

        Ok(output)
    }

    fn name(&self) -> Cow<'_, str> {
        "shutdown_response".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shutdown_response".to_string(),
            description: Some(
                "Respond to a shutdown request. Approve to shut down, reject to keep working."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "request_id": { "type": "string" },
                    "approve": { "type": "boolean" },
                    "reason": { "type": "string" }
                },
                "required": ["request_id", "approve"]
            }),
        }
    }
}

#[async_trait]
impl Tool for ShutdownResponseStatusTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let request_id = input
            .get("request_id")
            .and_then(|value| value.as_str())
            .context("Invalid request_id")?;
        self.manager.shutdown_status(request_id)
    }

    fn name(&self) -> Cow<'_, str> {
        "shutdown_response".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shutdown_response".to_string(),
            description: Some("Check the status of a shutdown request by request_id.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "request_id": { "type": "string" }
                },
                "required": ["request_id"]
            }),
        }
    }
}
