use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::store::{CollectionStore, StoreRoot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskStatus {
    Running,
    Completed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundTaskRecord {
    pub id: String,
    pub status: BackgroundTaskStatus,
    pub command: String,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub output: String,
}

#[derive(Debug)]
pub struct BackgroundManager {
    records: CollectionStore<BackgroundTaskRecord>,
    tasks: Mutex<HashMap<String, BackgroundTaskRecord>>,
    next_id: AtomicU64,
}

#[derive(Clone, Debug)]
pub struct SharedBackgroundManager {
    inner: Arc<BackgroundManager>,
}

impl SharedBackgroundManager {
    pub fn new(root: &StoreRoot) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(BackgroundManager {
                records: root.collection("background/tasks")?,
                tasks: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(Utc::now().timestamp_millis().max(0) as u64),
            }),
        })
    }

    pub fn run(&self, command: String) -> Result<String> {
        let id = format!("{:08x}", self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let record = BackgroundTaskRecord {
            id: id.clone(),
            status: BackgroundTaskStatus::Running,
            command: command.clone(),
            started_at: Utc::now(),
            finished_at: None,
            output: String::new(),
        };
        self.save_record(record.clone())?;

        let manager = self.clone();
        let command_for_task = command.clone();
        tokio::spawn(async move {
            let output = Command::new("sh")
                .arg("-c")
                .arg(&command_for_task)
                .output()
                .await;
            let mut record = record;
            record.finished_at = Some(Utc::now());
            match output {
                Ok(output) => {
                    record.status = if output.status.success() {
                        BackgroundTaskStatus::Completed
                    } else {
                        BackgroundTaskStatus::Error
                    };
                    record.output = format!(
                        "{}{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                Err(error) => {
                    record.status = BackgroundTaskStatus::Error;
                    record.output = error.to_string();
                }
            }
            let _ = manager.save_record(record);
        });

        Ok(format!("Background task {id} started: {command}"))
    }

    pub fn check(&self, task_id: Option<&str>) -> Result<String> {
        let tasks = self
            .inner
            .tasks
            .lock()
            .map_err(|_| anyhow::anyhow!("background manager lock poisoned"))?;

        if let Some(task_id) = task_id {
            let record = tasks
                .get(task_id)
                .cloned()
                .or_else(|| self.inner.records.read(task_id).ok())
                .with_context(|| format!("Unknown background task {task_id}"))?;
            return serde_json::to_string_pretty(&record).context("failed to serialize task");
        }

        if tasks.is_empty() {
            return Ok("No background tasks.".to_string());
        }
        let mut records = tasks.values().cloned().collect::<Vec<_>>();
        records.sort_by_key(|record| record.started_at);
        Ok(records
            .into_iter()
            .map(|record| format!("{}: {:?} {}", record.id, record.status, record.command))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn save_record(&self, record: BackgroundTaskRecord) -> Result<()> {
        self.inner.records.write(&record.id, &record)?;
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .map_err(|_| anyhow::anyhow!("background manager lock poisoned"))?;
        tasks.insert(record.id.clone(), record);
        Ok(())
    }
}

impl std::ops::Deref for SharedBackgroundManager {
    type Target = Arc<BackgroundManager>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
