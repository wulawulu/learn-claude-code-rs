use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use strum::EnumProperty;
use strum_macros::{Display, EnumProperty as EnumPropertyDerive, EnumString};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumString, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ClaimSource {
    Manual,
    Auto,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_source: Option<ClaimSource>,
    #[serde(
        rename = "claim_role",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub claim_role: Option<String>,
    #[serde(
        rename = "required_role",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_role: Option<String>,
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
            claimed_at: None,
            claim_source: None,
            claim_role: None,
            required_role: None,
        }
    }

    pub fn claim_role(&self) -> Option<&str> {
        self.claim_role
            .as_deref()
            .or(self.required_role.as_deref())
            .filter(|value| !value.is_empty())
    }

    pub fn allows_role(&self, role: Option<&str>) -> bool {
        match self.claim_role() {
            Some(required_role) => role.is_some_and(|role| role == required_role),
            None => true,
        }
    }

    pub fn is_claimable(&self, role: Option<&str>) -> bool {
        self.status == TaskStatus::Pending
            && self.owner.is_empty()
            && self.blocked_by.is_empty()
            && self.allows_role(role)
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
                    self.save(&blocked)?;
                }
            }
        }

        task.blocked_by.sort_unstable();
        task.blocks.sort_unstable();
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
                    format!(" (blocked by: {:?})", task.blocked_by)
                };
                let owner = if task.owner.is_empty() {
                    String::new()
                } else {
                    format!(" owner={}", task.owner)
                };
                let claim_role = task
                    .claim_role()
                    .map(|role| format!(" role={role}"))
                    .unwrap_or_default();
                format!(
                    "{} #{}: {}{}{}{}",
                    task.status.marker(),
                    task.id,
                    task.subject,
                    owner,
                    claim_role,
                    blocked
                )
            })
            .collect::<Vec<_>>();

        Ok(lines.join("\n"))
    }

    pub fn scan_unclaimed(&self, role: Option<&str>) -> Result<Vec<TaskRecord>> {
        let mut tasks = self.load_all()?;
        tasks.sort_by_key(|task| task.id);
        tasks.retain(|task| task.is_claimable(role));
        Ok(tasks)
    }

    pub fn claim(
        &mut self,
        task_id: u64,
        owner: &str,
        role: Option<&str>,
        source: ClaimSource,
    ) -> Result<String> {
        let mut task = self.load(task_id)?;
        if !task.is_claimable(role) {
            let role = role.unwrap_or("(any)");
            return Ok(format!(
                "Error: Task {task_id} is not claimable for role={role}"
            ));
        }

        task.owner = owner.to_string();
        task.status = TaskStatus::InProgress;
        task.claimed_at = Some(crate::team::unix_timestamp());
        task.claim_source = Some(source);
        self.save(&task)?;
        self.append_claim_event(task_id, owner, role, source)?;

        Ok(format!("Claimed task #{task_id} for {owner} via {source}"))
    }

    fn max_task_id(dir: &Path) -> Result<u64> {
        let mut max_id = 0;
        for entry in fs::read_dir(dir)
            .with_context(|| format!("failed to read task directory {}", dir.display()))?
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

    fn append_claim_event(
        &self,
        task_id: u64,
        owner: &str,
        role: Option<&str>,
        source: ClaimSource,
    ) -> Result<()> {
        let path = self.dir.join("claim_events.jsonl");
        let mut content = if path.exists() {
            fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?
        } else {
            String::new()
        };
        content.push_str(&serde_json::to_string(&json!({
            "event": "task.claimed",
            "task_id": task_id,
            "owner": owner,
            "role": role,
            "source": source,
            "ts": crate::team::unix_timestamp(),
        }))?);
        content.push('\n');
        fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))
    }

    fn clear_dependency(&self, completed_id: u64) -> Result<()> {
        for mut task in self.load_all()? {
            if task.blocked_by.contains(&completed_id) {
                task.blocked_by.retain(|id| *id != completed_id);
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

    pub fn update(&self, task_id: u64, update: TaskUpdate) -> Result<String> {
        self.with_manager(|manager| manager.update(task_id, update))
    }

    pub fn list_all(&self) -> Result<String> {
        self.with_manager(|manager| manager.list_all())
    }

    pub fn scan_unclaimed(&self, role: Option<&str>) -> Result<Vec<TaskRecord>> {
        self.with_manager(|manager| manager.scan_unclaimed(role))
    }

    pub fn claim(
        &self,
        task_id: u64,
        owner: &str,
        role: Option<&str>,
        source: ClaimSource,
    ) -> Result<String> {
        self.with_manager(|manager| manager.claim(task_id, owner, role, source))
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
