use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use strum_macros::Display;
use tokio::{process::Command, sync::mpsc, time::Duration};

pub const STALL_THRESHOLD_S: u64 = 45;
const BACKGROUND_TIMEOUT_S: u64 = 300;
const OUTPUT_LIMIT: usize = 50_000;
const PREVIEW_LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum BackgroundTaskStatus {
    Running,
    Completed,
    Timeout,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundTaskRecord {
    pub id: String,
    pub status: BackgroundTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    pub command: String,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub result_preview: String,
    pub output_file: String,
}

impl BackgroundTaskRecord {
    fn preview_text(&self) -> &str {
        if self.result_preview.is_empty() {
            "(running)"
        } else {
            &self.result_preview
        }
    }
}

impl fmt::Display for BackgroundTaskRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: [{}] {} -> {}",
            self.id,
            self.status,
            truncate_chars(&self.command, 60),
            self.preview_text()
        )
    }
}

#[derive(Debug, Serialize)]
struct BackgroundTaskView<'a> {
    id: &'a str,
    status: BackgroundTaskStatus,
    command: &'a str,
    started_at: &'a DateTime<Utc>,
    started_at_local: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<&'a DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at_local: Option<String>,
    result_preview: &'a str,
    output_file: &'a str,
}

impl<'a> From<&'a BackgroundTaskRecord> for BackgroundTaskView<'a> {
    fn from(record: &'a BackgroundTaskRecord) -> Self {
        Self {
            id: &record.id,
            status: record.status,
            command: &record.command,
            started_at: &record.started_at,
            started_at_local: format_local_time(&record.started_at),
            finished_at: record.finished_at.as_ref(),
            finished_at_local: record.finished_at.as_ref().map(format_local_time),
            result_preview: &record.result_preview,
            output_file: &record.output_file,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackgroundNotification {
    pub task_id: String,
    pub status: BackgroundTaskStatus,
    pub command: String,
    pub preview: String,
    pub output_file: String,
}

impl fmt::Display for BackgroundNotification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[bg:{}] {}: {} (command={}, output_file={})",
            self.task_id, self.status, self.preview, self.command, self.output_file
        )
    }
}

#[derive(Debug)]
struct BackgroundManager {
    dir: PathBuf,
    tasks: Mutex<HashMap<String, BackgroundTaskRecord>>,
    notification_tx: mpsc::UnboundedSender<BackgroundNotification>,
    notification_rx: Mutex<mpsc::UnboundedReceiver<BackgroundNotification>>,
    next_id: AtomicU64,
}

#[derive(Clone, Debug)]
pub struct SharedBackgroundManager {
    inner: Arc<BackgroundManager>,
}

impl SharedBackgroundManager {
    pub fn new(runtime_dir: impl AsRef<Path>) -> Result<Self> {
        let dir = runtime_dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create runtime directory {}", dir.display()))?;

        let (notification_tx, notification_rx) = mpsc::unbounded_channel();
        let seed = Utc::now().timestamp_millis().max(0) as u64;

        Ok(Self {
            inner: Arc::new(BackgroundManager {
                dir,
                tasks: Mutex::new(HashMap::new()),
                notification_tx,
                notification_rx: Mutex::new(notification_rx),
                next_id: AtomicU64::new(seed),
            }),
        })
    }

    pub fn run(&self, command: String) -> Result<String> {
        let task_id = format!("{:08x}", self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let output_file = self.output_path(&task_id);
        let record = BackgroundTaskRecord {
            id: task_id.clone(),
            status: BackgroundTaskStatus::Running,
            result: None,
            command: command.clone(),
            started_at: Utc::now(),
            finished_at: None,
            result_preview: String::new(),
            output_file: relative_output_path(&output_file),
        };

        self.with_tasks(|tasks| {
            tasks.insert(task_id.clone(), record.clone());
            Ok(())
        })?;
        self.persist_task(&record)?;

        let manager = self.clone();
        tokio::spawn(async move {
            manager.execute(task_id, command).await;
        });

        Ok(format!(
            "Background task {} started: {} (output_file={})",
            record.id,
            truncate_chars(&record.command, 80),
            record.output_file
        ))
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
                .with_context(|| format!("Unknown task {}", task_id))?;
            let view = BackgroundTaskView::from(record);
            return serde_json::to_string_pretty(&view).context("failed to serialize task");
        }

        if tasks.is_empty() {
            return Ok("No background tasks.".to_string());
        }

        let mut records = tasks.values().cloned().collect::<Vec<_>>();
        records.sort_by_key(|record| record.started_at);
        let lines = records
            .into_iter()
            .map(|record| record.to_string())
            .collect::<Vec<_>>();
        Ok(lines.join("\n"))
    }

    pub fn drain_notifications(&self) -> Vec<BackgroundNotification> {
        let Ok(mut receiver) = self.inner.notification_rx.lock() else {
            return Vec::new();
        };

        let mut notifications = Vec::new();
        while let Ok(notification) = receiver.try_recv() {
            notifications.push(notification);
        }
        notifications
    }

    pub fn background_results_message(&self) -> Option<String> {
        let notifications = self.drain_notifications();
        let stalled = self.detect_stalled();
        if notifications.is_empty() && stalled.is_empty() {
            return None;
        }

        let mut lines = notifications
            .into_iter()
            .map(|notification| notification.to_string())
            .collect::<Vec<_>>();

        if !stalled.is_empty() {
            lines.push(format!("stalled_tasks={}", stalled.join(", ")));
        }

        Some(format!(
            "<background-results>\n{}\n</background-results>",
            lines.join("\n")
        ))
    }

    pub fn detect_stalled(&self) -> Vec<String> {
        let now = Utc::now();
        let Ok(tasks) = self.inner.tasks.lock() else {
            return Vec::new();
        };

        tasks
            .values()
            .filter(|task| task.status == BackgroundTaskStatus::Running)
            .filter(|task| {
                now.signed_duration_since(task.started_at).num_seconds() > STALL_THRESHOLD_S as i64
            })
            .map(|task| task.id.clone())
            .collect()
    }

    async fn execute(&self, task_id: String, command: String) {
        let outcome = run_command(&command).await;
        let output_path = self.output_path(&task_id);
        let output_file = relative_output_path(&output_path);
        let preview = preview(&outcome.output);
        let record = BackgroundTaskRecord {
            id: task_id.clone(),
            status: outcome.status,
            result: Some(outcome.output.clone()),
            command: command.clone(),
            started_at: self.task_started_at(&task_id).unwrap_or_else(Utc::now),
            finished_at: Some(Utc::now()),
            result_preview: preview.clone(),
            output_file: output_file.clone(),
        };

        let _ = fs::write(&output_path, &outcome.output);
        let _ = self.with_tasks(|tasks| {
            tasks.insert(task_id.clone(), record.clone());
            Ok(())
        });
        let _ = self.persist_task(&record);
        let _ = self.inner.notification_tx.send(BackgroundNotification {
            task_id,
            status: outcome.status,
            command: truncate_chars(&command, 80),
            preview,
            output_file,
        });
    }

    fn persist_task(&self, task: &BackgroundTaskRecord) -> Result<()> {
        let path = self.record_path(&task.id);
        let payload = serde_json::to_string_pretty(task)?;
        fs::write(&path, payload)
            .with_context(|| format!("failed to write task record {}", path.display()))
    }

    fn task_started_at(&self, task_id: &str) -> Option<DateTime<Utc>> {
        self.inner
            .tasks
            .lock()
            .ok()
            .and_then(|tasks| tasks.get(task_id).map(|task| task.started_at))
    }

    fn with_tasks<T>(
        &self,
        callback: impl FnOnce(&mut HashMap<String, BackgroundTaskRecord>) -> Result<T>,
    ) -> Result<T> {
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .map_err(|_| anyhow::anyhow!("background manager lock poisoned"))?;
        callback(&mut tasks)
    }

    fn record_path(&self, task_id: &str) -> PathBuf {
        self.inner.dir.join(format!("{task_id}.json"))
    }

    fn output_path(&self, task_id: &str) -> PathBuf {
        self.inner.dir.join(format!("{task_id}.log"))
    }
}

#[derive(Debug)]
struct CommandOutcome {
    status: BackgroundTaskStatus,
    output: String,
}

impl CommandOutcome {
    fn error(output: String) -> Self {
        Self {
            status: BackgroundTaskStatus::Error,
            output,
        }
    }
}

async fn run_command(command: &str) -> CommandOutcome {
    let child = match Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return CommandOutcome::error(format!("Error: {}", error));
        }
    };

    match tokio::time::timeout(
        Duration::from_secs(BACKGROUND_TIMEOUT_S),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(output)) => {
            let combined = [output.stdout, output.stderr].concat();
            let text = String::from_utf8_lossy(&combined).trim().to_string();
            CommandOutcome {
                status: BackgroundTaskStatus::Completed,
                output: truncate_chars_owned(
                    if text.is_empty() {
                        "(no output)".to_string()
                    } else {
                        text
                    },
                    OUTPUT_LIMIT,
                ),
            }
        }
        Ok(Err(error)) => CommandOutcome::error(format!("Error: {}", error)),
        Err(_) => CommandOutcome {
            status: BackgroundTaskStatus::Timeout,
            output: "Error: Timeout (300s)".to_string(),
        },
    }
}

fn preview(output: &str) -> String {
    let compact = output.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&compact, PREVIEW_LIMIT)
}

fn format_local_time(timestamp: &DateTime<Utc>) -> String {
    timestamp
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S %:z")
        .to_string()
}

fn relative_output_path(output_path: &Path) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    output_path
        .strip_prefix(&cwd)
        .unwrap_or(output_path)
        .display()
        .to_string()
}

fn truncate_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn truncate_chars_owned(text: String, limit: usize) -> String {
    text.chars().take(limit).collect()
}
