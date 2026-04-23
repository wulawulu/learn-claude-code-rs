use std::{
    borrow::Cow,
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, task::SharedTaskManager, worktree::SharedWorktreeManager};

mod bash;
mod edit_file;
mod read_file;
mod sub_agent;
mod task_create;
mod task_get;
mod task_list;
mod task_update;
mod worktree;
mod write_file;

use self::{
    bash::bash_tool,
    edit_file::edit_file_tool,
    read_file::read_file_tool,
    sub_agent::sub_agent_tool,
    task_create::task_create_tool,
    task_get::task_get_tool,
    task_list::task_list_tool,
    task_update::task_update_tool,
    worktree::{
        worktree_closeout_tool, worktree_create_tool, worktree_enter_tool, worktree_events_tool,
        worktree_list_tool, worktree_run_tool, worktree_status_tool,
    },
    write_file::write_file_tool,
};

#[async_trait]
pub trait Tool: Send + Sync {
    async fn invoke(&mut self, input: &Value) -> Result<String>;
    fn name(&self) -> Cow<'_, str>;
    fn tool_spec(&self) -> ToolSpec;
}

pub fn toolset(
    tasks: SharedTaskManager,
    worktrees: SharedWorktreeManager,
    work_dir: PathBuf,
) -> HashMap<String, Box<dyn Tool>> {
    let mut tools = root_workspace_tools(work_dir.clone());
    tools.extend([
        (
            "sub_agent".to_string(),
            sub_agent_tool(worktrees.clone(), work_dir),
        ),
        ("task_create".to_string(), task_create_tool(tasks.clone())),
        ("task_get".to_string(), task_get_tool(tasks.clone())),
        ("task_list".to_string(), task_list_tool(tasks.clone())),
        ("task_update".to_string(), task_update_tool(tasks)),
        (
            "worktree_create".to_string(),
            worktree_create_tool(worktrees.clone()),
        ),
        (
            "worktree_list".to_string(),
            worktree_list_tool(worktrees.clone()),
        ),
        (
            "worktree_enter".to_string(),
            worktree_enter_tool(worktrees.clone()),
        ),
        (
            "worktree_status".to_string(),
            worktree_status_tool(worktrees.clone()),
        ),
        (
            "worktree_run".to_string(),
            worktree_run_tool(worktrees.clone()),
        ),
        (
            "worktree_closeout".to_string(),
            worktree_closeout_tool(worktrees.clone()),
        ),
        (
            "worktree_events".to_string(),
            worktree_events_tool(worktrees),
        ),
    ]);
    tools
}

pub fn subagent_toolset(work_dir: PathBuf) -> HashMap<String, Box<dyn Tool>> {
    lane_workspace_tools(work_dir)
}

fn root_workspace_tools(work_dir: PathBuf) -> HashMap<String, Box<dyn Tool>> {
    workspace_tools(work_dir, false)
}

fn lane_workspace_tools(work_dir: PathBuf) -> HashMap<String, Box<dyn Tool>> {
    workspace_tools(work_dir, true)
}

fn workspace_tools(
    work_dir: PathBuf,
    restrict_shell_paths: bool,
) -> HashMap<String, Box<dyn Tool>> {
    HashMap::from([
        (
            "bash".to_string(),
            bash_tool(work_dir.clone(), restrict_shell_paths),
        ),
        ("edit_file".to_string(), edit_file_tool(work_dir.clone())),
        ("read_file".to_string(), read_file_tool(work_dir.clone())),
        ("write_file".to_string(), write_file_tool(work_dir)),
    ])
}

pub(crate) fn safe_path(work_dir: &Path, path: &str) -> Result<PathBuf> {
    resolve_path(work_dir, path, false)
}

pub(crate) fn safe_path_allow_missing(work_dir: &Path, path: &str) -> Result<PathBuf> {
    resolve_path(work_dir, path, true)
}

fn resolve_path(work_dir: &Path, path: &str, allow_missing: bool) -> Result<PathBuf> {
    let candidate = Path::new(path);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        work_dir.join(candidate)
    };

    let full = if candidate.exists() || !allow_missing {
        candidate.canonicalize()?
    } else {
        let mut anchor = candidate.clone();
        let mut suffix = Vec::new();

        while !anchor.exists() {
            let name = anchor.file_name().context("Path has no file name")?;
            suffix.push(name.to_os_string());
            anchor = anchor.parent().context("Path has no parent")?.to_path_buf();
        }

        let mut resolved = anchor.canonicalize()?;
        if !resolved.starts_with(work_dir) {
            anyhow::bail!("path escapes scoped workspace {}", work_dir.display());
        }

        for component in suffix.into_iter().rev() {
            resolved.push(component);
        }
        resolved
    };

    if !full.starts_with(work_dir) {
        anyhow::bail!("path escapes scoped workspace {}", work_dir.display());
    }

    Ok(full)
}
