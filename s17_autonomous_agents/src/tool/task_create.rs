use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, task::SharedTaskManager, tool::Tool};

pub struct TaskCreateTool {
    manager: SharedTaskManager,
}

pub fn task_create_tool(manager: SharedTaskManager) -> Box<dyn Tool> {
    Box::new(TaskCreateTool { manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for TaskCreateTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let subject = input
            .get("subject")
            .and_then(|value| value.as_str())
            .context("Invalid subject")?
            .to_string();
        let description = input
            .get("description")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        self.manager.create(subject, description)
    }

    fn name(&self) -> Cow<'_, str> {
        "task_create".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "task_create".to_string(),
            description: Some("Create a new persistent task.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subject": { "type": "string" },
                    "description": { "type": "string" }
                },
                "required": ["subject"]
            }),
        }
    }
}
