use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, cron::SharedCronScheduler, tool::Tool};

pub struct CronDeleteTool {
    scheduler: SharedCronScheduler,
}

pub fn cron_delete_tool(scheduler: SharedCronScheduler) -> Box<dyn Tool> {
    Box::new(CronDeleteTool { scheduler }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for CronDeleteTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let id = input
            .get("id")
            .and_then(|value| value.as_str())
            .context("Invalid id")?;
        self.scheduler.delete(id)
    }

    fn name(&self) -> Cow<'_, str> {
        "cron_delete".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "cron_delete".to_string(),
            description: Some("Delete a scheduled task by ID.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Task ID to delete"
                    }
                },
                "required": ["id"]
            }),
        }
    }
}
