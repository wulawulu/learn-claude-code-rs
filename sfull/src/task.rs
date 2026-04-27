use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use strum::EnumProperty;
use strum_macros::{Display, EnumProperty as EnumPropertyDerive, EnumString};

use crate::store::{CollectionStore, Store, StoreRoot};

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
}

impl TaskRecord {
    pub fn new(id: u64, subject: String, description: Option<String>) -> Self {
        Self {
            id,
            subject,
            description,
            status: TaskStatus::Pending,
            blocked_by: Vec::new(),
            blocks: Vec::new(),
            owner: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskIndex {
    pub next_id: u64,
}

impl Default for TaskIndex {
    fn default() -> Self {
        Self { next_id: 1 }
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
    tasks: CollectionStore<TaskRecord>,
    index: Store<TaskIndex>,
}

impl TaskManager {
    pub fn new(root: &StoreRoot) -> Result<Self> {
        let manager = Self {
            tasks: root.collection("tasks")?,
            index: root.file("tasks/index.json")?,
        };
        if !manager.index.exists() {
            manager.index.write(&TaskIndex::default())?;
        }
        Ok(manager)
    }

    pub fn create(&mut self, subject: String, description: Option<String>) -> Result<TaskRecord> {
        let mut index = self.index.read().unwrap_or_default();
        let task = TaskRecord::new(index.next_id, subject, description);
        self.tasks.write(&task_key(task.id), &task)?;
        index.next_id += 1;
        self.index.write(&index)?;
        Ok(task)
    }

    pub fn get(&self, task_id: u64) -> Result<TaskRecord> {
        self.tasks
            .read(&task_key(task_id))
            .with_context(|| format!("Task {} not found", task_id))
    }

    pub fn update(&mut self, task_id: u64, update: TaskUpdate) -> Result<TaskRecord> {
        let mut task = self.get(task_id)?;

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
                if let Ok(mut blocked) = self.get(blocked_id)
                    && !blocked.blocked_by.contains(&task_id)
                {
                    blocked.blocked_by.push(task_id);
                    blocked.blocked_by.sort_unstable();
                    self.tasks.write(&task_key(blocked.id), &blocked)?;
                }
            }
        }

        task.blocked_by.sort_unstable();
        task.blocks.sort_unstable();
        self.tasks.write(&task_key(task.id), &task)?;
        Ok(task)
    }

    pub fn list(&self) -> Result<Vec<TaskRecord>> {
        let mut tasks = self.tasks.list()?;
        tasks.sort_by_key(|task| task.id);
        Ok(tasks)
    }

    pub fn delete(&mut self, task_id: u64) -> Result<TaskRecord> {
        let mut task = self.get(task_id)?;
        task.status = TaskStatus::Deleted;
        self.tasks.write(&task_key(task.id), &task)?;
        Ok(task)
    }

    fn clear_dependency(&self, completed_id: u64) -> Result<()> {
        for mut task in self.list()? {
            if task.blocked_by.contains(&completed_id) {
                task.blocked_by.retain(|id| *id != completed_id);
                self.tasks.write(&task_key(task.id), &task)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct SharedTaskManager {
    inner: Arc<Mutex<TaskManager>>,
}

impl SharedTaskManager {
    pub fn new(manager: TaskManager) -> Self {
        Self {
            inner: Arc::new(Mutex::new(manager)),
        }
    }

    pub fn create(&self, subject: String, description: Option<String>) -> Result<TaskRecord> {
        self.with_manager(|manager| manager.create(subject, description))
    }

    pub fn get(&self, task_id: u64) -> Result<TaskRecord> {
        self.with_manager(|manager| manager.get(task_id))
    }

    pub fn update(&self, task_id: u64, update: TaskUpdate) -> Result<TaskRecord> {
        self.with_manager(|manager| manager.update(task_id, update))
    }

    pub fn list(&self) -> Result<Vec<TaskRecord>> {
        self.with_manager(|manager| manager.list())
    }

    pub fn delete(&self, task_id: u64) -> Result<TaskRecord> {
        self.with_manager(|manager| manager.delete(task_id))
    }

    fn with_manager<T>(&self, callback: impl FnOnce(&mut TaskManager) -> Result<T>) -> Result<T> {
        let mut manager = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("task manager lock poisoned"))?;
        callback(&mut manager)
    }
}

impl std::ops::Deref for SharedTaskManager {
    type Target = Arc<Mutex<TaskManager>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub fn render_task_json(task: &TaskRecord) -> Result<String> {
    serde_json::to_string_pretty(task).context("failed to serialize task")
}

pub fn render_task_list(tasks: Vec<TaskRecord>) -> String {
    if tasks.is_empty() {
        return "No tasks.".to_string();
    }

    tasks
        .into_iter()
        .map(|task| {
            let blocked = if task.blocked_by.is_empty() {
                String::new()
            } else {
                format!(" (blocked by: {:?})", task.blocked_by)
            };
            let owner = if task.owner.is_empty() {
                String::new()
            } else {
                format!(" owner={}", task.owner)
            };
            format!(
                "{} #{}: {}{}{}",
                task.status.marker(),
                task.id,
                task.subject,
                owner,
                blocked
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn task_key(task_id: u64) -> String {
    format!("task_{task_id}")
}

fn merge_unique(target: &mut Vec<u64>, mut additions: Vec<u64>) {
    target.append(&mut additions);
    target.sort_unstable();
    target.dedup();
}
