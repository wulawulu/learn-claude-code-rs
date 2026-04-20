use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{
    ToolSpec,
    task::{ClaimSource, SharedTaskManager},
    tool::Tool,
};

pub struct TaskClaimTool {
    manager: SharedTaskManager,
    owner: String,
    role: Option<String>,
}

pub fn task_claim_tool(
    manager: SharedTaskManager,
    owner: impl Into<String>,
    role: Option<String>,
) -> Box<dyn Tool> {
    Box::new(TaskClaimTool {
        manager,
        owner: owner.into(),
        role,
    }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for TaskClaimTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let task_id = input
            .get("task_id")
            .and_then(|value| value.as_u64())
            .context("Invalid task_id")?;

        self.manager.claim(
            task_id,
            &self.owner,
            self.role.as_deref(),
            ClaimSource::Manual,
        )
    }

    fn name(&self) -> Cow<'_, str> {
        "claim_task".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "claim_task".to_string(),
            description: Some("Claim a ready task from the task board by ID.".to_string()),
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
