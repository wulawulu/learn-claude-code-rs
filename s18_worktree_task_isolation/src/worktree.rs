use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::now_ts;
use crate::task::{CloseoutAction, CloseoutRecord, SharedTaskManager, TaskStatus, WorktreeState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRecord {
    pub name: String,
    pub path: String,
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<u64>,
    pub status: WorktreeState,
    #[serde(default = "now_ts")]
    pub created_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_entered_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_command_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_command_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closeout: Option<CloseoutRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct WorktreeIndex {
    #[serde(default)]
    worktrees: Vec<WorktreeRecord>,
}

#[derive(Debug)]
pub struct EventBus {
    path: PathBuf,
}

impl EventBus {
    fn new(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        if !path.exists() {
            fs::write(&path, "").with_context(|| format!("failed to create {}", path.display()))?;
        }
        Ok(Self { path })
    }

    fn emit(
        &self,
        event: &str,
        task_id: Option<u64>,
        worktree: Option<&str>,
        extra: Value,
    ) -> Result<()> {
        let mut payload = Map::from_iter([
            ("event".to_string(), Value::String(event.to_string())),
            ("ts".to_string(), json!(now_ts())),
        ]);

        if let Some(task_id) = task_id {
            payload.insert("task_id".to_string(), json!(task_id));
        }
        if let Some(worktree) = worktree {
            payload.insert("worktree".to_string(), Value::String(worktree.to_string()));
        }
        if let Value::Object(extra) = extra {
            payload.extend(extra);
        }

        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .with_context(|| format!("failed to open {}", self.path.display()))?;
        writeln!(file, "{}", serde_json::to_string(&payload)?)
            .with_context(|| format!("failed to append {}", self.path.display()))?;
        Ok(())
    }

    fn emit_worktree(&self, event: &str, task_id: Option<u64>, worktree: &str) -> Result<()> {
        self.emit(event, task_id, Some(worktree), json!({}))
    }

    fn emit_worktree_with(
        &self,
        event: &str,
        task_id: Option<u64>,
        worktree: &str,
        extra: Value,
    ) -> Result<()> {
        self.emit(event, task_id, Some(worktree), extra)
    }

    fn emit_worktree_error(
        &self,
        event: &str,
        task_id: Option<u64>,
        worktree: &str,
        error: &dyn std::fmt::Display,
    ) -> Result<()> {
        self.emit_worktree_with(
            event,
            task_id,
            worktree,
            json!({ "error": error.to_string() }),
        )
    }

    fn list_recent(&self, limit: usize) -> Result<String> {
        let limit = limit.clamp(1, 200);
        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        let items = content
            .lines()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| {
                serde_json::from_str::<Value>(line)
                    .unwrap_or_else(|_| json!({"event":"parse_error","raw":line}))
            })
            .collect::<Vec<_>>();

        serde_json::to_string_pretty(&items).context("failed to render events")
    }
}

#[derive(Debug)]
pub struct WorktreeManager {
    repo_root: PathBuf,
    dir: PathBuf,
    index_path: PathBuf,
    tasks: SharedTaskManager,
    events: EventBus,
    git_available: bool,
}

impl WorktreeManager {
    pub fn new(repo_root: impl AsRef<Path>, tasks: SharedTaskManager) -> Result<Self> {
        let repo_root = repo_root.as_ref().to_path_buf();
        let dir = repo_root.join(".worktrees");
        fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;

        let index_path = dir.join("index.json");
        if !index_path.exists() {
            fs::write(
                &index_path,
                serde_json::to_string_pretty(&WorktreeIndex::default())?,
            )
            .with_context(|| format!("failed to create {}", index_path.display()))?;
        }

        let events = EventBus::new(dir.join("events.jsonl"))?;
        let git_available = Self::check_git(&repo_root);

        Ok(Self {
            repo_root,
            dir,
            index_path,
            tasks,
            events,
            git_available,
        })
    }

    pub fn git_available(&self) -> bool {
        self.git_available
    }

    pub fn create(&mut self, name: &str, task_id: Option<u64>, base_ref: &str) -> Result<String> {
        self.validate_name(name)?;
        if self.find(name)?.is_some() {
            anyhow::bail!("Worktree '{name}' already exists");
        }
        if let Some(task_id) = task_id
            && !self.tasks.exists(task_id)
        {
            anyhow::bail!("Task {} not found", task_id);
        }

        let path = self.dir.join(name);
        let branch = format!("wt/{name}");
        self.events
            .emit_worktree("worktree.create.before", task_id, name)?;

        let result = (|| -> Result<String> {
            self.run_git([
                "worktree",
                "add",
                "-b",
                &branch,
                &path.display().to_string(),
                base_ref,
            ])?;

            let record = WorktreeRecord {
                name: name.to_string(),
                path: self.relative_display(&path),
                branch,
                task_id,
                status: WorktreeState::Active,
                created_at: now_ts(),
                last_entered_at: None,
                last_command_at: None,
                last_command_preview: None,
                closeout: None,
            };

            let mut index = self.load_index()?;
            index.worktrees.push(record.clone());
            self.save_index(&index)?;

            if let Some(task_id) = task_id {
                self.tasks.bind_worktree(task_id, name.to_string(), None)?;
            }
            self.events
                .emit_worktree("worktree.create.after", task_id, name)?;

            serde_json::to_string_pretty(&record).context("failed to render worktree")
        })();

        if let Err(error) = &result {
            let _ = self
                .events
                .emit_worktree_error("worktree.create.failed", task_id, name, error);
        }

        result
    }

    pub fn list_all(&self) -> Result<String> {
        let index = self.load_index()?;
        if index.worktrees.is_empty() {
            return Ok("No worktrees in index.".to_string());
        }

        let mut lines = Vec::new();
        for worktree in index.worktrees {
            let task = worktree
                .task_id
                .map(|task_id| format!(" task={task_id}"))
                .unwrap_or_default();
            lines.push(format!(
                "[{}] {} -> {} ({}){}",
                worktree.status, worktree.name, worktree.path, worktree.branch, task
            ));
        }
        Ok(lines.join("\n"))
    }

    pub fn status(&self, name: &str) -> Result<String> {
        let record = self
            .find(name)?
            .with_context(|| format!("Unknown worktree '{name}'"))?;
        let path = self.absolute_record_path(&record)?;
        self.run_git_in(&path, ["status", "--short", "--branch"])
    }

    pub fn enter(&mut self, name: &str) -> Result<String> {
        let task_id = self.find(name)?.and_then(|record| record.task_id);
        let updated = self.update_entry(name, |record| {
            record.last_entered_at = Some(now_ts());
        })?;
        self.events.emit_worktree_with(
            "worktree.enter",
            task_id,
            name,
            json!({ "path": updated.path }),
        )?;
        serde_json::to_string_pretty(&updated).context("failed to render worktree")
    }

    pub fn run(&mut self, name: &str, command: &str) -> Result<String> {
        guard_dangerous_command(command)?;

        let record = self
            .find(name)?
            .with_context(|| format!("Unknown worktree '{name}'"))?;
        let path = self.absolute_record_path(&record)?;
        let preview = truncate_preview(command, 120);

        self.update_entry(name, |item| {
            item.last_entered_at = Some(now_ts());
            item.last_command_at = Some(now_ts());
            item.last_command_preview = Some(preview.clone());
        })?;

        self.events.emit_worktree_with(
            "worktree.run.before",
            record.task_id,
            name,
            json!({ "command": preview }),
        )?;

        let output = self.run_shell_in(&path, command).inspect_err(|error| {
            let _ =
                self.events
                    .emit_worktree_error("worktree.run.failed", record.task_id, name, error);
        })?;

        self.events
            .emit_worktree("worktree.run.after", record.task_id, name)?;

        Ok(output)
    }

    pub fn closeout(
        &mut self,
        name: &str,
        action: CloseoutAction,
        reason: &str,
        force: bool,
        complete_task: bool,
    ) -> Result<String> {
        match action {
            CloseoutAction::Keep => self.keep(name, reason, complete_task),
            CloseoutAction::Remove => self.remove(name, force, complete_task, reason),
        }
    }

    pub fn keep(&mut self, name: &str, reason: &str, complete_task: bool) -> Result<String> {
        let record = self
            .find(name)?
            .with_context(|| format!("Unknown worktree '{name}'"))?;

        if let Some(task_id) = record.task_id {
            self.tasks
                .record_closeout(task_id, CloseoutAction::Keep, reason.to_string(), true)?;
            if complete_task {
                self.tasks.update(
                    task_id,
                    crate::task::TaskUpdate {
                        status: Some(TaskStatus::Completed),
                        ..Default::default()
                    },
                )?;
            }
        }

        let closeout = CloseoutRecord::new(CloseoutAction::Keep, reason.to_string());
        let updated = self.update_entry(name, |item| {
            item.status = WorktreeState::Kept;
            item.closeout = Some(closeout.clone());
        })?;

        self.events.emit_worktree_with(
            "worktree.closeout.keep",
            record.task_id,
            name,
            json!({ "reason": reason, "complete_task": complete_task }),
        )?;

        serde_json::to_string_pretty(&updated).context("failed to render worktree")
    }

    pub fn remove(
        &mut self,
        name: &str,
        force: bool,
        complete_task: bool,
        reason: &str,
    ) -> Result<String> {
        let record = self
            .find(name)?
            .with_context(|| format!("Unknown worktree '{name}'"))?;
        let path = self.absolute_record_path(&record)?;

        self.events.emit_worktree_with(
            "worktree.remove.before",
            record.task_id,
            name,
            json!({ "force": force }),
        )?;

        let result = (|| -> Result<String> {
            if !force && self.is_dirty(&path)? {
                anyhow::bail!(
                    "Worktree '{name}' has uncommitted changes. Use force=true to remove"
                );
            }

            let mut args = vec!["worktree".to_string(), "remove".to_string()];
            if force {
                args.push("--force".to_string());
            }
            args.push(path.display().to_string());
            self.run_git(args.iter().map(String::as_str))?;

            if let Some(task_id) = record.task_id {
                self.tasks.record_closeout(
                    task_id,
                    CloseoutAction::Remove,
                    reason.to_string(),
                    false,
                )?;

                if complete_task {
                    self.tasks.update(
                        task_id,
                        crate::task::TaskUpdate {
                            status: Some(TaskStatus::Completed),
                            ..Default::default()
                        },
                    )?;
                }
            }

            let closeout = CloseoutRecord::new(CloseoutAction::Remove, reason.to_string());
            self.update_entry(name, |item| {
                item.status = WorktreeState::Removed;
                item.closeout = Some(closeout.clone());
            })?;

            self.events.emit_worktree_with(
                "worktree.remove.after",
                record.task_id,
                name,
                json!({ "reason": reason, "complete_task": complete_task }),
            )?;
            Ok(format!("Removed worktree '{name}'"))
        })();

        if let Err(error) = &result {
            let _ = self.events.emit_worktree_error(
                "worktree.remove.failed",
                record.task_id,
                name,
                error,
            );
        }

        result
    }

    pub fn events(&self, limit: usize) -> Result<String> {
        self.events.list_recent(limit)
    }

    pub fn get_record(&self, name: &str) -> Result<WorktreeRecord> {
        self.find(name)?
            .with_context(|| format!("Unknown worktree '{name}'"))
    }

    pub fn path_for(&self, name: &str) -> Result<PathBuf> {
        let record = self.get_record(name)?;
        self.absolute_record_path(&record)
    }

    pub fn path_for_task(&self, task_id: u64) -> Result<(String, PathBuf)> {
        let task = self.tasks.get_record(task_id)?;
        if !task.worktree.is_empty() {
            let path = self.path_for(&task.worktree)?;
            return Ok((task.worktree, path));
        }

        let index = self.load_index()?;
        let record = index
            .worktrees
            .into_iter()
            .rev()
            .find(|record| record.task_id == Some(task_id))
            .with_context(|| format!("Task {task_id} has no worktree binding"))?;

        let path = self.absolute_record_path(&record)?;
        Ok((record.name, path))
    }

    pub fn record_event(
        &self,
        event: &str,
        task_id: Option<u64>,
        worktree: Option<&str>,
        extra: Value,
    ) -> Result<()> {
        self.events.emit(event, task_id, worktree, extra)
    }

    pub fn record_subagent_event(
        &self,
        event: &str,
        task_id: Option<u64>,
        worktree: &str,
        description: Option<&str>,
    ) -> Result<()> {
        self.events.emit_worktree_with(
            event,
            task_id,
            worktree,
            json!({
                "description": description.unwrap_or_default(),
            }),
        )
    }

    fn check_git(repo_root: &Path) -> bool {
        Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(repo_root)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn validate_name(&self, name: &str) -> Result<()> {
        let valid = Regex::new(r"^[A-Za-z0-9._-]{1,40}$")?;
        if valid.is_match(name) {
            Ok(())
        } else {
            anyhow::bail!("Invalid worktree name. Use 1-40 chars: letters, digits, ., _, -")
        }
    }

    fn run_git<'a>(&self, args: impl IntoIterator<Item = &'a str>) -> Result<String> {
        if !self.git_available {
            anyhow::bail!("Not in a git repository.");
        }

        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo_root)
            .output()
            .context("failed to spawn git")?;

        if !output.status.success() {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            anyhow::bail!(combined.trim().to_string());
        }

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let combined = combined.trim();
        if combined.is_empty() {
            Ok("(no output)".to_string())
        } else {
            Ok(combined.to_string())
        }
    }

    fn run_git_in<'a>(
        &self,
        cwd: &Path,
        args: impl IntoIterator<Item = &'a str>,
    ) -> Result<String> {
        if !self.git_available {
            anyhow::bail!("Not in a git repository.");
        }

        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .context("failed to spawn git")?;

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            anyhow::bail!(combined.trim().to_string());
        }

        let combined = combined.trim();
        if combined.is_empty() {
            Ok("Clean worktree".to_string())
        } else {
            Ok(combined.to_string())
        }
    }

    fn run_shell_in(&self, cwd: &Path, command: &str) -> Result<String> {
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .output()
            .context("failed to spawn shell")?;

        let combined = [output.stdout, output.stderr].concat();
        let text = String::from_utf8_lossy(&combined).trim().to_string();
        if output.status.success() {
            Ok(if text.is_empty() {
                "(no output)".to_string()
            } else {
                text.chars().take(50_000).collect()
            })
        } else {
            anyhow::bail!(if text.is_empty() {
                format!("Command failed: {command}")
            } else {
                text
            })
        }
    }

    fn is_dirty(&self, cwd: &Path) -> Result<bool> {
        let output = self.run_git_in(cwd, ["status", "--porcelain"])?;
        Ok(!output.trim().is_empty() && output.trim() != "Clean worktree")
    }

    fn load_index(&self) -> Result<WorktreeIndex> {
        let content = fs::read_to_string(&self.index_path)
            .with_context(|| format!("failed to read {}", self.index_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", self.index_path.display()))
    }

    fn save_index(&self, index: &WorktreeIndex) -> Result<()> {
        fs::write(&self.index_path, serde_json::to_string_pretty(index)?)
            .with_context(|| format!("failed to write {}", self.index_path.display()))?;
        Ok(())
    }

    fn find(&self, name: &str) -> Result<Option<WorktreeRecord>> {
        Ok(self
            .load_index()?
            .worktrees
            .into_iter()
            .find(|record| record.name == name))
    }

    fn update_entry(
        &mut self,
        name: &str,
        update: impl FnOnce(&mut WorktreeRecord),
    ) -> Result<WorktreeRecord> {
        let mut index = self.load_index()?;
        let record = index
            .worktrees
            .iter_mut()
            .find(|record| record.name == name)
            .with_context(|| format!("Worktree '{name}' not found in index"))?;
        update(record);
        let updated = record.clone();
        self.save_index(&index)?;
        Ok(updated)
    }

    fn absolute_record_path(&self, record: &WorktreeRecord) -> Result<PathBuf> {
        let path = PathBuf::from(&record.path);
        let path = if path.is_absolute() {
            path
        } else {
            self.repo_root.join(path)
        };
        Ok(path)
    }

    fn relative_display(&self, path: &Path) -> String {
        path.strip_prefix(&self.repo_root)
            .ok()
            .map(|relative| relative.display().to_string())
            .unwrap_or_else(|| path.display().to_string())
    }
}

#[derive(Clone, Debug)]
pub struct SharedWorktreeManager {
    inner: Arc<Mutex<WorktreeManager>>,
}

impl SharedWorktreeManager {
    pub fn new(repo_root: impl AsRef<Path>, tasks: SharedTaskManager) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Mutex::new(WorktreeManager::new(repo_root, tasks)?)),
        })
    }

    pub fn git_available(&self) -> bool {
        self.with_manager(|manager| Ok(manager.git_available()))
            .unwrap_or(false)
    }

    pub fn create(&self, name: &str, task_id: Option<u64>, base_ref: &str) -> Result<String> {
        self.with_manager(|manager| manager.create(name, task_id, base_ref))
    }

    pub fn list_all(&self) -> Result<String> {
        self.with_manager(|manager| manager.list_all())
    }

    pub fn status(&self, name: &str) -> Result<String> {
        self.with_manager(|manager| manager.status(name))
    }

    pub fn enter(&self, name: &str) -> Result<String> {
        self.with_manager(|manager| manager.enter(name))
    }

    pub fn run(&self, name: &str, command: &str) -> Result<String> {
        self.with_manager(|manager| manager.run(name, command))
    }

    pub fn closeout(
        &self,
        name: &str,
        action: CloseoutAction,
        reason: &str,
        force: bool,
        complete_task: bool,
    ) -> Result<String> {
        self.with_manager(|manager| manager.closeout(name, action, reason, force, complete_task))
    }

    pub fn events(&self, limit: usize) -> Result<String> {
        self.with_manager(|manager| manager.events(limit))
    }

    pub fn get_record(&self, name: &str) -> Result<WorktreeRecord> {
        self.with_manager(|manager| manager.get_record(name))
    }

    pub fn path_for(&self, name: &str) -> Result<PathBuf> {
        self.with_manager(|manager| manager.path_for(name))
    }

    pub fn path_for_task(&self, task_id: u64) -> Result<(String, PathBuf)> {
        self.with_manager(|manager| manager.path_for_task(task_id))
    }

    pub fn record_event(
        &self,
        event: &str,
        task_id: Option<u64>,
        worktree: Option<&str>,
        extra: Value,
    ) -> Result<()> {
        self.with_manager(|manager| manager.record_event(event, task_id, worktree, extra))
    }

    pub fn record_subagent_event(
        &self,
        event: &str,
        task_id: Option<u64>,
        worktree: &str,
        description: Option<&str>,
    ) -> Result<()> {
        self.with_manager(|manager| {
            manager.record_subagent_event(event, task_id, worktree, description)
        })
    }

    fn with_manager<T>(
        &self,
        callback: impl FnOnce(&mut WorktreeManager) -> Result<T>,
    ) -> Result<T> {
        let mut manager = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("worktree manager lock poisoned"))?;
        callback(&mut manager)
    }
}

fn guard_dangerous_command(command: &str) -> Result<()> {
    let dangerous = ["rm -rf /", "sudo", "shutdown", "reboot", "> /dev/"];
    if dangerous.iter().any(|item| command.contains(item)) {
        anyhow::bail!("Error: Dangerous command blocked");
    }
    Ok(())
}

fn truncate_preview(command: &str, limit: usize) -> String {
    command.chars().take(limit).collect()
}
