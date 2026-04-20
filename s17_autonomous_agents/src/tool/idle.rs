use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, tool::Tool};

pub struct IdleTool {
    message: String,
}

pub fn idle_tool(message: impl Into<String>) -> Box<dyn Tool> {
    Box::new(IdleTool {
        message: message.into(),
    }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for IdleTool {
    async fn invoke(&mut self, _input: &Value) -> Result<String> {
        Ok(self.message.clone())
    }

    fn name(&self) -> Cow<'_, str> {
        "idle".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "idle".to_string(),
            description: Some(
                "Signal that you have no more immediate work and want to enter idle polling."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }
}
