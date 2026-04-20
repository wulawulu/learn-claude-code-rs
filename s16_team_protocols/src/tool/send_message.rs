use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{
    ToolSpec,
    team::{MessageType, SharedTeammateManager},
    tool::Tool,
};

pub struct SendMessageTool {
    manager: SharedTeammateManager,
    sender_name: String,
}

pub fn send_message_tool(
    manager: SharedTeammateManager,
    sender_name: impl Into<String>,
) -> Box<dyn Tool> {
    Box::new(SendMessageTool {
        manager,
        sender_name: sender_name.into(),
    }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for SendMessageTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let to = input
            .get("to")
            .and_then(|value| value.as_str())
            .context("Invalid to")?;
        let content = input
            .get("content")
            .and_then(|value| value.as_str())
            .context("Invalid content")?;
        let message_type = input
            .get("msg_type")
            .and_then(|value| value.as_str())
            .map(str::parse)
            .transpose()?
            .unwrap_or(MessageType::Message);

        self.manager
            .send_message(&self.sender_name, to, content, message_type)
    }

    fn name(&self) -> Cow<'_, str> {
        "send_message".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "send_message".to_string(),
            description: Some("Send a message to a teammate's inbox.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string" },
                    "content": { "type": "string" },
                    "msg_type": {
                        "type": "string",
                        "enum": [
                            "message",
                            "broadcast",
                            "shutdown_request",
                            "shutdown_response",
                            "plan_approval",
                            "plan_approval_response"
                        ]
                    }
                },
                "required": ["to", "content"]
            }),
        }
    }
}
