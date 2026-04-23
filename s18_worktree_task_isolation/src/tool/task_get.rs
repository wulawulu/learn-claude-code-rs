use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, task::SharedTaskManager, tool::Tool};

pub struct TaskGetTool {
    manager: SharedTaskManager,
}

pub fn task_get_tool(manager: SharedTaskManager) -> Box<dyn Tool> {
    Box::new(TaskGetTool { manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for TaskGetTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let task_id = input
            .get("task_id")
            .and_then(|value| value.as_u64())
            .context("Invalid task_id")?;
        self.manager.get(task_id)
    }

    fn name(&self) -> Cow<'_, str> {
        "task_get".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "task_get".to_string(),
            description: Some("Get full details of a task by ID.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "integer" }
                },
                "required": ["task_id"]
            }),
        }
    }
}
