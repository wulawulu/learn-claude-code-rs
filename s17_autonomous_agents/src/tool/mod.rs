use std::borrow::Cow;
use std::collections::HashMap;

use crate::team::SharedTeammateManager;
use crate::{ToolSpec, task::SharedTaskManager};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

mod bash;
mod broadcast;
mod edit_file;
mod idle;
mod list_teammates;
mod plan_approval;
mod read_file;
mod read_inbox;
mod send_message;
mod shutdown_request;
mod shutdown_response;
mod spawn_teammate;
mod task_claim;
mod task_create;
mod task_get;
mod task_list;
mod task_update;
mod write_file;

use bash::bash_tool;
use broadcast::broadcast_tool;
use edit_file::edit_file_tool;
use idle::idle_tool;
use list_teammates::list_teammates_tool;
use plan_approval::{plan_approval_review_tool, plan_approval_submit_tool};
use read_file::read_file_tool;
use read_inbox::read_inbox_tool;
use send_message::send_message_tool;
use shutdown_request::shutdown_request_tool;
use shutdown_response::{shutdown_response_status_tool, shutdown_response_tool};
use spawn_teammate::spawn_teammate_tool;
use task_claim::task_claim_tool;
use task_create::task_create_tool;
use task_get::task_get_tool;
use task_list::task_list_tool;
use task_update::task_update_tool;
use write_file::write_file_tool;

pub fn leader_tools(
    manager: SharedTeammateManager,
    tasks: SharedTaskManager,
) -> HashMap<String, Box<dyn Tool>> {
    HashMap::from([
        ("bash".to_string(), bash_tool()),
        (
            "broadcast".to_string(),
            broadcast_tool(manager.clone(), "lead"),
        ),
        ("edit_file".to_string(), edit_file_tool()),
        ("idle".to_string(), idle_tool("Lead does not idle.")),
        (
            "list_teammates".to_string(),
            list_teammates_tool(manager.clone()),
        ),
        ("read_file".to_string(), read_file_tool()),
        (
            "read_inbox".to_string(),
            read_inbox_tool(manager.clone(), "lead"),
        ),
        (
            "send_message".to_string(),
            send_message_tool(manager.clone(), "lead"),
        ),
        (
            "spawn_teammate".to_string(),
            spawn_teammate_tool(manager.clone()),
        ),
        (
            "shutdown_request".to_string(),
            shutdown_request_tool(manager.clone()),
        ),
        (
            "shutdown_response".to_string(),
            shutdown_response_status_tool(manager.clone()),
        ),
        (
            "plan_approval".to_string(),
            plan_approval_review_tool(manager),
        ),
        (
            "claim_task".to_string(),
            task_claim_tool(tasks.clone(), "lead", None),
        ),
        ("task_create".to_string(), task_create_tool(tasks.clone())),
        ("task_get".to_string(), task_get_tool(tasks.clone())),
        ("task_list".to_string(), task_list_tool(tasks.clone())),
        ("task_update".to_string(), task_update_tool(tasks)),
        ("write_file".to_string(), write_file_tool()),
    ])
}

pub struct TeammateToolsInput {
    pub tasks: SharedTaskManager,
    pub sender_name: String,
    pub role: String,
    pub manager: SharedTeammateManager,
}

pub fn teammate_tools_input(
    manager: SharedTeammateManager,
    tasks: SharedTaskManager,
    sender_name: impl Into<String>,
    role: impl Into<String>,
) -> TeammateToolsInput {
    TeammateToolsInput {
        tasks,
        manager,
        sender_name: sender_name.into(),
        role: role.into(),
    }
}

pub fn teammate_tools(input: TeammateToolsInput) -> HashMap<String, Box<dyn Tool>> {
    let TeammateToolsInput {
        tasks,
        manager,
        sender_name,
        role,
    } = input;

    HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("write_file".to_string(), write_file_tool()),
        ("edit_file".to_string(), edit_file_tool()),
        (
            "send_message".to_string(),
            send_message_tool(manager.clone(), sender_name.clone()),
        ),
        (
            "read_inbox".to_string(),
            read_inbox_tool(manager.clone(), sender_name.clone()),
        ),
        (
            "shutdown_response".to_string(),
            shutdown_response_tool(manager.clone(), sender_name.clone()),
        ),
        (
            "plan_approval".to_string(),
            plan_approval_submit_tool(manager, sender_name.clone()),
        ),
        (
            "claim_task".to_string(),
            task_claim_tool(tasks, sender_name, Some(role)),
        ),
        (
            "idle".to_string(),
            idle_tool("Entering idle phase. Will poll for new tasks."),
        ),
    ])
}

#[async_trait]
pub trait Tool: Send + Sync {
    async fn invoke(&mut self, input: &Value) -> Result<String>;
    fn name(&self) -> Cow<'_, str>;
    fn tool_spec(&self) -> ToolSpec;
}

fn safe_path(path: &str) -> Result<std::path::PathBuf> {
    resolve_safe_path(path, false)
}

fn safe_path_allow_missing(path: &str) -> Result<std::path::PathBuf> {
    resolve_safe_path(path, true)
}

fn resolve_safe_path(path: &str, allow_missing: bool) -> Result<std::path::PathBuf> {
    let cwd = std::env::current_dir()?;
    let candidate = cwd.join(path);

    let full = if candidate.exists() || !allow_missing {
        candidate.canonicalize()?
    } else {
        let parent = candidate
            .parent()
            .context("Path has no parent")?
            .canonicalize()?;

        if !parent.starts_with(&cwd) {
            return Err(anyhow::anyhow!("Path escapes workspace"));
        }

        let file_name = candidate.file_name().context("Path has no file name")?;

        parent.join(file_name)
    };

    if !full.starts_with(&cwd) {
        return Err(anyhow::anyhow!("Path escapes workspace"));
    }

    Ok(full)
}
