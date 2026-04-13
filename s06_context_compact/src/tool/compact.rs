use std::borrow::Cow;

use crate::{ToolSpec, tool::Tool};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

pub struct CompactTool;

pub fn compact_tool() -> Box<dyn Tool> {
    Box::new(CompactTool {}) as Box<dyn Tool>
}

#[async_trait]
impl Tool for CompactTool {
    async fn invoke(&mut self, _input: &Value) -> Result<String> {
        Ok("Compacting conversation...".into())
    }

    fn name(&self) -> Cow<'_, str> {
        "compact".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "compact".to_string(),
            description: Some(
                "Summarize earlier conversation so work can continue in a smaller context."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "focus": {"type": "string"},
                },
            }),
        }
    }
}
