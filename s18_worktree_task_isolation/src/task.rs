use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use strum::EnumProperty;
use strum_macros::{Display, EnumProperty as EnumPropertyDerive, EnumString};

use crate::now_ts;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    EnumString,
    Display,
    EnumPropertyDerive,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TaskStatus {
    #[strum(props(marker = "[ ]"))]
    Pending,
    #[strum(props(marker = "[>]"))]
    InProgress,
    #[strum(props(marker = "[x]"))]
    Completed,
    #[strum(props(marker = "[-]"))]
    Deleted,
}

impl TaskStatus {
    pub fn marker(self) -> &'static str {
        self.get_str("marker").unwrap_or("[?]")
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumString, Display, Default,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WorktreeState {
    #[default]
    Unbound,
    Active,
    Kept,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumString, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum CloseoutAction {
    Keep,
    Remove,
}

impl CloseoutAction {
    pub fn state(self) -> WorktreeState {
        match self {
            Self::Keep => WorktreeState::Kept,
            Self::Remove => WorktreeState::Removed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseoutRecord {
    pub action: CloseoutAction,
    #[serde(default)]
    pub reason: String,
    pub at: f64,
}

impl CloseoutRecord {
    pub fn new(action: CloseoutAction, reason: impl Into<String>) -> Self {
        Self {
            action,
            reason: reason.into(),
            at: now_ts(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: u64,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: TaskStatus,
    #[serde(rename = "blockedBy", default)]
    pub blocked_by: Vec<u64>,
    #[serde(default)]
    pub blocks: Vec<u64>,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default)]
    pub worktree_state: WorktreeState,
    #[serde(default)]
    pub last_worktree: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closeout: Option<CloseoutRecord>,
    #[serde(default = "now_ts")]
    pub created_at: f64,
    #[serde(default = "now_ts")]
    pub updated_at: f64,
}

impl TaskRecord {
    pub fn new(id: u64, subject: String, description: Option<String>) -> Self {
        let now = now_ts();
        Self {
            id,
            subject,
            description,
            status: TaskStatus::Pending,
            blocked_by: Vec::new(),
            blocks: Vec::new(),
            owner: String::new(),
            worktree: String::new(),
            worktree_state: WorktreeState::Unbound,
            last_worktree: String::new(),
            closeout: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskUpdate {
    pub status: Option<TaskStatus>,
    pub owner: Option<String>,
    pub add_blocked_by: Vec<u64>,
    pub add_blocks: Vec<u64>,
}

#[derive(Debug)]
pub struct TaskManager {
    dir: PathBuf,
    next_id: u64,
}

impl TaskManager {
    pub fn new(tasks_dir: impl AsRef<Path>) -> Result<Self> {
        let dir = tasks_dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create tasks directory {}", dir.display()))?;

        let next_id = Self::max_task_id(&dir)? + 1;
        Ok(Self { dir, next_id })
    }

    pub fn create(&mut self, subject: String, description: Option<String>) -> Result<String> {
        let task = TaskRecord::new(self.next_id, subject, description);
        self.save(&task)?;
        self.next_id += 1;
        self.render_json(&task)
    }

    pub fn get(&self, task_id: u64) -> Result<String> {
        let task = self.load(task_id)?;
        self.render_json(&task)
    }

    pub fn get_record(&self, task_id: u64) -> Result<TaskRecord> {
        self.load(task_id)
    }

    pub fn exists(&self, task_id: u64) -> bool {
        self.task_path(task_id).exists()
    }

    pub fn update(&mut self, task_id: u64, update: TaskUpdate) -> Result<String> {
        let mut task = self.load(task_id)?;

        if let Some(owner) = update.owner {
            task.owner = owner;
        }

        if let Some(status) = update.status {
            task.status = status;
            if status == TaskStatus::Completed {
                self.clear_dependency(task_id)?;
            }
        }

        if !update.add_blocked_by.is_empty() {
            merge_unique(&mut task.blocked_by, update.add_blocked_by);
        }

        if !update.add_blocks.is_empty() {
            merge_unique(&mut task.blocks, update.add_blocks.clone());
            for blocked_id in update.add_blocks {
                if let Ok(mut blocked) = self.load(blocked_id)
                    && !blocked.blocked_by.contains(&task_id)
                {
                    blocked.blocked_by.push(task_id);
                    blocked.blocked_by.sort_unstable();
                    blocked.updated_at = now_ts();
                    self.save(&blocked)?;
                }
            }
        }

        task.blocked_by.sort_unstable();
        task.blocks.sort_unstable();
        task.updated_at = now_ts();
        self.save(&task)?;
        self.render_json(&task)
    }

    pub fn bind_worktree(
        &mut self,
        task_id: u64,
        worktree: impl Into<String>,
        owner: Option<String>,
    ) -> Result<String> {
        let mut task = self.load(task_id)?;
        let worktree = worktree.into();

        task.worktree = worktree.clone();
        task.last_worktree = worktree;
        task.worktree_state = WorktreeState::Active;
        task.closeout = None;

        if let Some(owner) = owner {
            task.owner = owner;
        }
        if task.status == TaskStatus::Pending {
            task.status = TaskStatus::InProgress;
        }
        task.updated_at = now_ts();
        self.save(&task)?;
        self.render_json(&task)
    }

    pub fn unbind_worktree(&mut self, task_id: u64) -> Result<String> {
        let mut task = self.load(task_id)?;
        task.worktree.clear();
        task.worktree_state = WorktreeState::Unbound;
        task.updated_at = now_ts();
        self.save(&task)?;
        self.render_json(&task)
    }

    pub fn record_closeout(
        &mut self,
        task_id: u64,
        action: CloseoutAction,
        reason: impl Into<String>,
        keep_binding: bool,
    ) -> Result<String> {
        let mut task = self.load(task_id)?;
        task.closeout = Some(CloseoutRecord::new(action, reason));
        task.worktree_state = action.state();
        if !keep_binding {
            task.worktree.clear();
        }
        task.updated_at = now_ts();
        self.save(&task)?;
        self.render_json(&task)
    }

    pub fn list_all(&self) -> Result<String> {
        let mut tasks = self.load_all()?;
        if tasks.is_empty() {
            return Ok("No tasks.".to_string());
        }

        tasks.sort_by_key(|task| task.id);
        let lines = tasks
            .into_iter()
            .map(|task| {
                let blocked = if task.blocked_by.is_empty() {
                    String::new()
                } else {
                    format!(" blocked_by={:?}", task.blocked_by)
                };
                let owner = if task.owner.is_empty() {
                    String::new()
                } else {
                    format!(" owner={}", task.owner)
                };
                let worktree = if task.worktree.is_empty() {
                    if task.last_worktree.is_empty() {
                        String::new()
                    } else {
                        format!(" last_wt={}({})", task.last_worktree, task.worktree_state)
                    }
                } else {
                    format!(" wt={}({})", task.worktree, task.worktree_state)
                };

                format!(
                    "{} #{}: {}{}{}{}",
                    task.status.marker(),
                    task.id,
                    task.subject,
                    owner,
                    worktree,
                    blocked
                )
            })
            .collect::<Vec<_>>();

        Ok(lines.join("\n"))
    }

    fn max_task_id(dir: &Path) -> Result<u64> {
        let mut max_id = 0;
        for entry in
            fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };

            let Some(id_text) = name
                .strip_prefix("task_")
                .and_then(|value| value.strip_suffix(".json"))
            else {
                continue;
            };

            let Ok(id) = id_text.parse::<u64>() else {
                continue;
            };
            max_id = max_id.max(id);
        }
        Ok(max_id)
    }

    fn load(&self, task_id: u64) -> Result<TaskRecord> {
        let path = self.task_path(task_id);
        let content =
            fs::read_to_string(&path).with_context(|| format!("Task {} not found", task_id))?;
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse task file {}", path.display()))
    }

    fn load_all(&self) -> Result<Vec<TaskRecord>> {
        let mut tasks = Vec::new();
        for entry in fs::read_dir(&self.dir)
            .with_context(|| format!("failed to read task directory {}", self.dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.starts_with("task_") || !name.ends_with(".json") {
                continue;
            }

            let content = fs::read_to_string(&path)?;
            let task: TaskRecord = serde_json::from_str(&content)
                .with_context(|| format!("failed to parse task file {}", path.display()))?;
            tasks.push(task);
        }
        Ok(tasks)
    }

    fn save(&self, task: &TaskRecord) -> Result<()> {
        let path = self.task_path(task.id);
        let content = serde_json::to_string_pretty(task)?;
        fs::write(&path, content)
            .with_context(|| format!("failed to write task file {}", path.display()))?;
        Ok(())
    }

    fn clear_dependency(&self, completed_id: u64) -> Result<()> {
        for mut task in self.load_all()? {
            if task.blocked_by.contains(&completed_id) {
                task.blocked_by.retain(|id| *id != completed_id);
                task.updated_at = now_ts();
                self.save(&task)?;
            }
        }
        Ok(())
    }

    fn render_json(&self, task: &TaskRecord) -> Result<String> {
        serde_json::to_string_pretty(task).context("failed to serialize task")
    }

    fn task_path(&self, task_id: u64) -> PathBuf {
        self.dir.join(format!("task_{task_id}.json"))
    }
}

#[derive(Clone, Debug)]
pub struct SharedTaskManager {
    inner: Arc<Mutex<TaskManager>>,
}

impl SharedTaskManager {
    pub fn new(tasks_dir: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Mutex::new(TaskManager::new(tasks_dir)?)),
        })
    }

    pub fn create(&self, subject: String, description: Option<String>) -> Result<String> {
        self.with_manager(|manager| manager.create(subject, description))
    }

    pub fn get(&self, task_id: u64) -> Result<String> {
        self.with_manager(|manager| manager.get(task_id))
    }

    pub fn get_record(&self, task_id: u64) -> Result<TaskRecord> {
        self.with_manager(|manager| manager.get_record(task_id))
    }

    pub fn exists(&self, task_id: u64) -> bool {
        self.with_manager(|manager| Ok(manager.exists(task_id)))
            .unwrap_or(false)
    }

    pub fn update(&self, task_id: u64, update: TaskUpdate) -> Result<String> {
        self.with_manager(|manager| manager.update(task_id, update))
    }

    pub fn bind_worktree(
        &self,
        task_id: u64,
        worktree: impl Into<String>,
        owner: Option<String>,
    ) -> Result<String> {
        self.with_manager(|manager| manager.bind_worktree(task_id, worktree, owner))
    }

    pub fn unbind_worktree(&self, task_id: u64) -> Result<String> {
        self.with_manager(|manager| manager.unbind_worktree(task_id))
    }

    pub fn record_closeout(
        &self,
        task_id: u64,
        action: CloseoutAction,
        reason: impl Into<String>,
        keep_binding: bool,
    ) -> Result<String> {
        let reason = reason.into();
        self.with_manager(|manager| manager.record_closeout(task_id, action, reason, keep_binding))
    }

    pub fn list_all(&self) -> Result<String> {
        self.with_manager(|manager| manager.list_all())
    }

    fn with_manager<T>(&self, callback: impl FnOnce(&mut TaskManager) -> Result<T>) -> Result<T> {
        let mut manager = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("task manager lock poisoned"))?;
        callback(&mut manager)
    }
}

fn merge_unique(target: &mut Vec<u64>, mut additions: Vec<u64>) {
    target.append(&mut additions);
    target.sort_unstable();
    target.dedup();
}
