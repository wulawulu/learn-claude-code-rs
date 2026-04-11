use std::{borrow::Cow, str};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use strum::EnumProperty as _;
use strum_macros::EnumProperty;

use crate::{ToolSpec, tool::Tool};

#[derive(EnumProperty, PartialEq, Eq, Clone, Debug, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanItemStatus {
    #[strum(props(marker = "[ ]"))]
    Pending,
    #[strum(props(marker = "[>]"))]
    InProgress,
    #[strum(props(marker = "[x]"))]
    Completed,
}

impl PlanItemStatus {
    pub fn marker(&self) -> &'static str {
        self.get_str("marker").unwrap()
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct PlanItem {
    content: String,
    status: PlanItemStatus,
    #[serde(rename = "activeForm")]
    active_form: Option<String>,
}

impl std::fmt::Display for PlanItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(active_form) = self.active_form.as_ref()
            && self.status == PlanItemStatus::InProgress
        {
            write!(
                f,
                "{} {} ({})",
                self.status.marker(),
                self.content,
                active_form
            )
        } else {
            write!(f, "{} {}", self.status.marker(), self.content,)
        }
    }
}

pub fn todo_tool() -> Box<dyn Tool> {
    Box::new(TodoManager::new()) as Box<dyn Tool>
}

#[derive(Default)]
pub struct TodoManager {
    items: Vec<PlanItem>,
}

impl TodoManager {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn update(&mut self, items: Vec<PlanItem>) -> Result<String> {
        if items.len() > 12 {
            return Err(anyhow::anyhow!(
                "Keep the session plan short (max 12 items)"
            ));
        }
        // count how many items are in progress
        let in_progress_count = items
            .iter()
            .filter(|item| matches!(item.status, PlanItemStatus::InProgress))
            .count();
        if in_progress_count > 1 {
            return Err(anyhow::anyhow!("Only one plan item can be in_progress"));
        }
        self.items = items;
        Ok(self.render())
    }

    pub fn render(&self) -> String {
        if self.items.is_empty() {
            return "No session plan yet.".into();
        }
        let items = self
            .items
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let completed = self
            .items
            .iter()
            .filter(|item| matches!(item.status, PlanItemStatus::Completed))
            .count();
        let total = self.items.len();
        let rendered = format!("{}\n({}/{} completed)", items, completed, total);
        rendered
    }
}

#[async_trait]
impl Tool for TodoManager {
    fn name(&self) -> Cow<'_, str> {
        "todo".into()
    }

    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let items_value = if let Some(items) = input.get("items") {
            items.clone()
        } else if input.is_array() {
            input.clone()
        } else {
            return Err(anyhow::anyhow!("Invalid items"));
        };

        let items: Vec<PlanItem> =
            serde_json::from_value(items_value).context("deserialize plan item failed")?;
        self.update(items)
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "todo".to_string(),
            description: Some("Rewrite the current session plan for multi-step work.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {"type": "string"},
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                },
                                "activeForm": {
                                    "type": "string",
                                    "description": "Optional present-continuous label.",
                                },
                            },
                            "required": ["content", "status"],
                        },
                    },
                },
                "required": ["items"],
            }),
        }
    }
}
