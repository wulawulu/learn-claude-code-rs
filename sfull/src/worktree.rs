use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::store::{Store, StoreRoot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRecord {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub task_id: Option<u64>,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorktreeIndex {
    #[serde(default)]
    pub worktrees: Vec<WorktreeRecord>,
    #[serde(default)]
    pub events: Vec<String>,
}

#[derive(Debug)]
pub struct WorktreeManager {
    repo_root: PathBuf,
    index: Store<WorktreeIndex>,
}

#[derive(Clone, Debug)]
pub struct SharedWorktreeManager {
    inner: Arc<Mutex<WorktreeManager>>,
}

impl WorktreeManager {
    pub fn new(root: &StoreRoot, repo_root: PathBuf) -> Result<Self> {
        let manager = Self {
            repo_root,
            index: root.file("worktrees/index.json")?,
        };
        if !manager.index.exists() {
            manager.index.write(&WorktreeIndex::default())?;
        }
        Ok(manager)
    }

    pub fn create(
        &mut self,
        name: String,
        task_id: Option<u64>,
        base_ref: String,
    ) -> Result<String> {
        let mut index = self.index.read().unwrap_or_default();
        if index.worktrees.iter().any(|worktree| worktree.name == name) {
            anyhow::bail!("worktree {name} already exists");
        }
        let dir = self.repo_root.join(".worktrees").join(&name);
        let branch = format!("wt/{name}");
        let status = Command::new("git")
            .current_dir(&self.repo_root)
            .args([
                "worktree",
                "add",
                "-b",
                &branch,
                &dir.display().to_string(),
                &base_ref,
            ])
            .status();
        if let Ok(status) = status
            && !status.success()
        {
            anyhow::bail!("git worktree add failed with {status}");
        }
        let record = WorktreeRecord {
            name: name.clone(),
            path: dir.display().to_string(),
            branch,
            task_id,
            status: "active".to_string(),
        };
        index.worktrees.push(record.clone());
        index
            .events
            .push(format!("{} worktree.create {}", Utc::now(), name));
        self.index.write(&index)?;
        serde_json::to_string_pretty(&record).context("failed to serialize worktree")
    }

    pub fn list(&self) -> Result<String> {
        let index = self.index.read().unwrap_or_default();
        if index.worktrees.is_empty() {
            return Ok("No worktrees.".to_string());
        }
        Ok(index
            .worktrees
            .into_iter()
            .map(|worktree| format!("{} {} {}", worktree.name, worktree.branch, worktree.path))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn status(&self, name: &str) -> Result<String> {
        let record = self.find(name)?;
        let output = Command::new("git")
            .current_dir(&record.path)
            .arg("status")
            .output()
            .with_context(|| format!("failed to run git status in {}", record.path))?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub fn run(&mut self, name: &str, command: &str) -> Result<String> {
        let record = self.find(name)?;
        let output = Command::new("sh")
            .current_dir(&record.path)
            .arg("-c")
            .arg(command)
            .output()
            .with_context(|| format!("failed to run command in {}", record.path))?;
        Ok(format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }

    pub fn events(&self, limit: usize) -> Result<String> {
        let index = self.index.read().unwrap_or_default();
        Ok(index
            .events
            .into_iter()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn find(&self, name: &str) -> Result<WorktreeRecord> {
        self.index
            .read()
            .unwrap_or_default()
            .worktrees
            .into_iter()
            .find(|worktree| worktree.name == name)
            .with_context(|| format!("worktree {name} not found"))
    }
}

impl SharedWorktreeManager {
    pub fn new(manager: WorktreeManager) -> Self {
        Self {
            inner: Arc::new(Mutex::new(manager)),
        }
    }

    pub fn with_manager<T>(&self, f: impl FnOnce(&mut WorktreeManager) -> Result<T>) -> Result<T> {
        let mut manager = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("worktree manager lock poisoned"))?;
        f(&mut manager)
    }

    pub fn create(&self, name: String, task_id: Option<u64>, base_ref: String) -> Result<String> {
        self.with_manager(|manager| manager.create(name, task_id, base_ref))
    }

    pub fn list(&self) -> Result<String> {
        self.with_manager(|manager| manager.list())
    }

    pub fn status(&self, name: &str) -> Result<String> {
        self.with_manager(|manager| manager.status(name))
    }

    pub fn run(&self, name: &str, command: &str) -> Result<String> {
        self.with_manager(|manager| manager.run(name, command))
    }

    pub fn events(&self, limit: usize) -> Result<String> {
        self.with_manager(|manager| manager.events(limit))
    }
}

impl std::ops::Deref for SharedWorktreeManager {
    type Target = Arc<Mutex<WorktreeManager>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
