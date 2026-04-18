use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, cron::SharedCronScheduler, tool::Tool};

pub struct CronListTool {
    scheduler: SharedCronScheduler,
}

pub fn cron_list_tool(scheduler: SharedCronScheduler) -> Box<dyn Tool> {
    Box::new(CronListTool { scheduler }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for CronListTool {
    async fn invoke(&mut self, _input: &Value) -> Result<String> {
        self.scheduler.list_tasks()
    }

    fn name(&self) -> Cow<'_, str> {
        "cron_list".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "cron_list".to_string(),
            description: Some("List all scheduled tasks.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }
}
