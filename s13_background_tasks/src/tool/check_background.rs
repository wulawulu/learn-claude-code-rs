use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, background::SharedBackgroundManager, tool::Tool};

pub struct CheckBackgroundTool {
    manager: SharedBackgroundManager,
}

pub fn check_background_tool(manager: SharedBackgroundManager) -> Box<dyn Tool> {
    Box::new(CheckBackgroundTool { manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for CheckBackgroundTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        self.manager
            .check(input.get("task_id").and_then(|value| value.as_str()))
    }

    fn name(&self) -> Cow<'_, str> {
        "check_background".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "check_background".to_string(),
            description: Some(
                "Check one background task or list every background task.".to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" }
                }
            }),
        }
    }
}
