use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::store::{Store, StoreRoot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTaskRecord {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub recurring: bool,
    pub durable: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScheduledTaskIndex {
    #[serde(default)]
    pub tasks: Vec<ScheduledTaskRecord>,
    #[serde(default)]
    pub next_id: u64,
}

#[derive(Debug)]
pub struct CronScheduler {
    store: Store<ScheduledTaskIndex>,
}

#[derive(Clone, Debug)]
pub struct SharedCronScheduler {
    inner: Arc<Mutex<CronScheduler>>,
}

impl CronScheduler {
    pub fn new(root: &StoreRoot) -> Result<Self> {
        let scheduler = Self {
            store: root.file("cron/scheduled_tasks.json")?,
        };
        if !scheduler.store.exists() {
            scheduler.store.write(&ScheduledTaskIndex::default())?;
        }
        Ok(scheduler)
    }

    pub fn create(
        &mut self,
        cron: String,
        prompt: String,
        recurring: bool,
        durable: bool,
    ) -> Result<ScheduledTaskRecord> {
        let mut index = self.store.read().unwrap_or_default();
        let id_num = index.next_id;
        index.next_id += 1;
        let task = ScheduledTaskRecord {
            id: format!("{id_num:08x}"),
            cron,
            prompt,
            recurring,
            durable,
            created_at: Utc::now().timestamp(),
        };
        index.tasks.push(task.clone());
        self.store.write(&index)?;
        Ok(task)
    }

    pub fn delete(&mut self, id: &str) -> Result<String> {
        let mut index = self.store.read().unwrap_or_default();
        let before = index.tasks.len();
        index.tasks.retain(|task| task.id != id);
        if index.tasks.len() == before {
            anyhow::bail!("scheduled task {id} not found");
        }
        self.store.write(&index)?;
        Ok(format!("Deleted scheduled task {id}"))
    }

    pub fn list(&self) -> Result<String> {
        let mut tasks = self.store.read().unwrap_or_default().tasks;
        if tasks.is_empty() {
            return Ok("No scheduled tasks.".to_string());
        }
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(tasks
            .into_iter()
            .map(|task| {
                format!(
                    "{} {} [{}{}]: {}",
                    task.id,
                    task.cron,
                    if task.recurring {
                        "recurring"
                    } else {
                        "one-shot"
                    },
                    if task.durable { "/durable" } else { "/session" },
                    task.prompt
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

impl SharedCronScheduler {
    pub fn new(scheduler: CronScheduler) -> Self {
        Self {
            inner: Arc::new(Mutex::new(scheduler)),
        }
    }

    pub fn create(
        &self,
        cron: String,
        prompt: String,
        recurring: bool,
        durable: bool,
    ) -> Result<String> {
        let task =
            self.with_scheduler(|scheduler| scheduler.create(cron, prompt, recurring, durable))?;
        serde_json::to_string_pretty(&task).context("failed to serialize scheduled task")
    }

    pub fn delete(&self, id: &str) -> Result<String> {
        self.with_scheduler(|scheduler| scheduler.delete(id))
    }

    pub fn list(&self) -> Result<String> {
        self.with_scheduler(|scheduler| scheduler.list())
    }

    fn with_scheduler<T>(&self, f: impl FnOnce(&mut CronScheduler) -> Result<T>) -> Result<T> {
        let mut scheduler = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("cron scheduler lock poisoned"))?;
        f(&mut scheduler)
    }
}

impl std::ops::Deref for SharedCronScheduler {
    type Target = Arc<Mutex<CronScheduler>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
