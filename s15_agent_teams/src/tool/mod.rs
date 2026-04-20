use std::borrow::Cow;
use std::collections::HashMap;

use crate::ToolSpec;
use crate::team::{SharedMessageBus, SharedTeammateManager};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

mod bash;
mod broadcast;
mod edit_file;
mod list_teammates;
mod read_file;
mod read_inbox;
mod send_message;
mod spawn_teammate;
mod write_file;

use bash::bash_tool;
use broadcast::broadcast_tool;
use edit_file::edit_file_tool;
use list_teammates::list_teammates_tool;
use read_file::read_file_tool;
use read_inbox::read_inbox_tool;
use send_message::send_message_tool;
use spawn_teammate::spawn_teammate_tool;
use write_file::write_file_tool;

pub fn leader_tools(
    bus: SharedMessageBus,
    manager: SharedTeammateManager,
) -> HashMap<String, Box<dyn Tool>> {
    HashMap::from([
        ("bash".to_string(), bash_tool()),
        (
            "broadcast".to_string(),
            broadcast_tool(bus.clone(), manager.clone()),
        ),
        ("edit_file".to_string(), edit_file_tool()),
        (
            "list_teammates".to_string(),
            list_teammates_tool(manager.clone()),
        ),
        ("read_file".to_string(), read_file_tool()),
        (
            "read_inbox".to_string(),
            read_inbox_tool(bus.clone(), "lead"),
        ),
        (
            "send_message".to_string(),
            send_message_tool(bus.clone(), "lead"),
        ),
        (
            "spawn_teammate".to_string(),
            spawn_teammate_tool(manager.clone()),
        ),
        ("write_file".to_string(), write_file_tool()),
    ])
}

pub fn teammate_tools(
    bus: SharedMessageBus,
    sender_name: impl Into<String>,
) -> HashMap<String, Box<dyn Tool>> {
    let sender_name = sender_name.into();

    HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("write_file".to_string(), write_file_tool()),
        ("edit_file".to_string(), edit_file_tool()),
        (
            "send_message".to_string(),
            send_message_tool(bus.clone(), sender_name.clone()),
        ),
        (
            "read_inbox".to_string(),
            read_inbox_tool(bus.clone(), sender_name),
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
