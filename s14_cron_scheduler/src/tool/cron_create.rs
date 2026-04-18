use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, cron::SharedCronScheduler, tool::Tool};

pub struct CronCreateTool {
    scheduler: SharedCronScheduler,
}

pub fn cron_create_tool(scheduler: SharedCronScheduler) -> Box<dyn Tool> {
    Box::new(CronCreateTool { scheduler }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for CronCreateTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let cron = input
            .get("cron")
            .and_then(|value| value.as_str())
            .context("Invalid cron")?;
        let prompt = input
            .get("prompt")
            .and_then(|value| value.as_str())
            .context("Invalid prompt")?;
        let recurring = input
            .get("recurring")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let durable = input
            .get("durable")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        self.scheduler.create(cron, prompt, recurring, durable)
    }

    fn name(&self) -> Cow<'_, str> {
        "cron_create".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "cron_create".to_string(),
            description: Some(
                "Schedule a recurring or one-shot task with a cron expression.".to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "cron": {
                        "type": "string",
                        "description": "Cron expression understood by the scheduler"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The prompt to inject when the task fires"
                    },
                    "recurring": {
                        "type": "boolean",
                        "description": "true=repeat, false=fire once then delete. Default true."
                    },
                    "durable": {
                        "type": "boolean",
                        "description": "true=persist to disk, false=session-only. Default false."
                    }
                },
                "required": ["cron", "prompt"]
            }),
        }
    }
}
