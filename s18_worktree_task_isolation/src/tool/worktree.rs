use std::{borrow::Cow, str::FromStr};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, task::CloseoutAction, tool::Tool, worktree::SharedWorktreeManager};

pub fn worktree_create_tool(manager: SharedWorktreeManager) -> Box<dyn Tool> {
    Box::new(WorktreeCreateTool { manager })
}

pub fn worktree_list_tool(manager: SharedWorktreeManager) -> Box<dyn Tool> {
    Box::new(WorktreeListTool { manager })
}

pub fn worktree_enter_tool(manager: SharedWorktreeManager) -> Box<dyn Tool> {
    Box::new(WorktreeEnterTool { manager })
}

pub fn worktree_status_tool(manager: SharedWorktreeManager) -> Box<dyn Tool> {
    Box::new(WorktreeStatusTool { manager })
}

pub fn worktree_run_tool(manager: SharedWorktreeManager) -> Box<dyn Tool> {
    Box::new(WorktreeRunTool { manager })
}

pub fn worktree_closeout_tool(manager: SharedWorktreeManager) -> Box<dyn Tool> {
    Box::new(WorktreeCloseoutTool { manager })
}

pub fn worktree_events_tool(manager: SharedWorktreeManager) -> Box<dyn Tool> {
    Box::new(WorktreeEventsTool { manager })
}

struct WorktreeCreateTool {
    manager: SharedWorktreeManager,
}

struct WorktreeListTool {
    manager: SharedWorktreeManager,
}

struct WorktreeEnterTool {
    manager: SharedWorktreeManager,
}

struct WorktreeStatusTool {
    manager: SharedWorktreeManager,
}

struct WorktreeRunTool {
    manager: SharedWorktreeManager,
}

struct WorktreeCloseoutTool {
    manager: SharedWorktreeManager,
}

struct WorktreeEventsTool {
    manager: SharedWorktreeManager,
}

#[async_trait]
impl Tool for WorktreeCreateTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let name = input
            .get("name")
            .and_then(|value| value.as_str())
            .context("Invalid name")?;
        let task_id = input.get("task_id").and_then(|value| value.as_u64());
        let base_ref = input
            .get("base_ref")
            .and_then(|value| value.as_str())
            .unwrap_or("HEAD");

        self.manager.create(name, task_id, base_ref)
    }

    fn name(&self) -> Cow<'_, str> {
        "worktree_create".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "worktree_create".to_string(),
            description: Some(
                "Create an isolated git worktree lane and bind it to a task.".to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "task_id": { "type": "integer" },
                    "base_ref": { "type": "string" }
                },
                "required": ["name"]
            }),
        }
    }
}

#[async_trait]
impl Tool for WorktreeListTool {
    async fn invoke(&mut self, _: &Value) -> Result<String> {
        self.manager.list_all()
    }

    fn name(&self) -> Cow<'_, str> {
        "worktree_list".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "worktree_list".to_string(),
            description: Some("List tracked worktree lanes.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }
}

#[async_trait]
impl Tool for WorktreeEnterTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let name = input
            .get("name")
            .and_then(|value| value.as_str())
            .context("Invalid name")?;
        self.manager.enter(name)
    }

    fn name(&self) -> Cow<'_, str> {
        "worktree_enter".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "worktree_enter".to_string(),
            description: Some("Mark a worktree lane as entered before doing work.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "required": ["name"]
            }),
        }
    }
}

#[async_trait]
impl Tool for WorktreeStatusTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let name = input
            .get("name")
            .and_then(|value| value.as_str())
            .context("Invalid name")?;
        self.manager.status(name)
    }

    fn name(&self) -> Cow<'_, str> {
        "worktree_status".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "worktree_status".to_string(),
            description: Some("Show git status for a worktree lane.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "required": ["name"]
            }),
        }
    }
}

#[async_trait]
impl Tool for WorktreeRunTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let name = input
            .get("name")
            .and_then(|value| value.as_str())
            .context("Invalid name")?;
        let command = input
            .get("command")
            .and_then(|value| value.as_str())
            .context("Invalid command")?;

        self.manager.run(name, command)
    }

    fn name(&self) -> Cow<'_, str> {
        "worktree_run".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "worktree_run".to_string(),
            description: Some("Run one shell command inside a named worktree.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "command": { "type": "string" }
                },
                "required": ["name", "command"]
            }),
        }
    }
}

#[async_trait]
impl Tool for WorktreeCloseoutTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let name = input
            .get("name")
            .and_then(|value| value.as_str())
            .context("Invalid name")?;
        let action = input
            .get("action")
            .and_then(|value| value.as_str())
            .context("Invalid action")?;
        let action = CloseoutAction::from_str(action)
            .map_err(|_| anyhow::anyhow!("Invalid action. Use keep or remove"))?;
        let reason = input
            .get("reason")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let force = input
            .get("force")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let complete_task = input
            .get("complete_task")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        self.manager
            .closeout(name, action, reason, force, complete_task)
    }

    fn name(&self) -> Cow<'_, str> {
        "worktree_closeout".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "worktree_closeout".to_string(),
            description: Some("Close out a worktree by keeping it or removing it.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "action": { "type": "string", "enum": ["keep", "remove"] },
                    "reason": { "type": "string" },
                    "force": { "type": "boolean" },
                    "complete_task": { "type": "boolean" }
                },
                "required": ["name", "action"]
            }),
        }
    }
}

#[async_trait]
impl Tool for WorktreeEventsTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let limit = input
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(20) as usize;
        self.manager.events(limit)
    }

    fn name(&self) -> Cow<'_, str> {
        "worktree_events".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "worktree_events".to_string(),
            description: Some("List recent worktree lifecycle events.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer" }
                }
            }),
        }
    }
}
