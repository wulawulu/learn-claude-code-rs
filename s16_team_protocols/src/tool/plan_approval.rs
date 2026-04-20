use std::borrow::Cow;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, team::SharedTeammateManager, tool::Tool};

pub struct PlanApprovalSubmitTool {
    manager: SharedTeammateManager,
    sender_name: String,
}

pub struct PlanApprovalReviewTool {
    manager: SharedTeammateManager,
}

pub fn plan_approval_submit_tool(
    manager: SharedTeammateManager,
    sender_name: impl Into<String>,
) -> Box<dyn Tool> {
    Box::new(PlanApprovalSubmitTool {
        manager,
        sender_name: sender_name.into(),
    }) as Box<dyn Tool>
}

pub fn plan_approval_review_tool(manager: SharedTeammateManager) -> Box<dyn Tool> {
    Box::new(PlanApprovalReviewTool { manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for PlanApprovalSubmitTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let plan = input
            .get("plan")
            .and_then(|value| value.as_str())
            .context("Invalid plan")?;

        self.manager.submit_plan(&self.sender_name, plan)
    }

    fn name(&self) -> Cow<'_, str> {
        "plan_approval".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "plan_approval".to_string(),
            description: Some("Submit a plan for lead approval. Provide plan text.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "plan": { "type": "string" }
                },
                "required": ["plan"]
            }),
        }
    }
}

#[async_trait]
impl Tool for PlanApprovalReviewTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let request_id = input
            .get("request_id")
            .and_then(|value| value.as_str())
            .context("Invalid request_id")?;
        let approve = input
            .get("approve")
            .and_then(|value| value.as_bool())
            .context("Invalid approve")?;
        let feedback = input
            .get("feedback")
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        self.manager.review_plan(request_id, approve, feedback)
    }

    fn name(&self) -> Cow<'_, str> {
        "plan_approval".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "plan_approval".to_string(),
            description: Some(
                "Approve or reject a teammate's plan. Provide request_id + approve + optional feedback."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "request_id": { "type": "string" },
                    "approve": { "type": "boolean" },
                    "feedback": { "type": "string" }
                },
                "required": ["request_id", "approve"]
            }),
        }
    }
}
