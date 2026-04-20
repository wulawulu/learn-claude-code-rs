use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, task::SharedTaskManager, tool::Tool};

pub struct TaskListTool {
    manager: SharedTaskManager,
}

pub fn task_list_tool(manager: SharedTaskManager) -> Box<dyn Tool> {
    Box::new(TaskListTool { manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for TaskListTool {
    async fn invoke(&mut self, _input: &Value) -> Result<String> {
        self.manager.list_all()
    }

    fn name(&self) -> Cow<'_, str> {
        "task_list".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "task_list".to_string(),
            description: Some("List all tasks with status summary.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }
}
