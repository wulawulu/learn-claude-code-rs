use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    fmt, fs,
    fs::{File, OpenOptions},
    hash::{Hash, Hasher},
    io::ErrorKind,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Local, Timelike, Utc};
use cron::Schedule;
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};
use tokio::{
    task::JoinHandle,
    time::{Duration, interval},
};

const AUTO_EXPIRY_DAYS: i64 = 7;
const CHECK_INTERVAL_SECS: u64 = 1;
const JITTER_OFFSET_MAX_MINUTES: u64 = 4;
const JITTER_TARGET_MINUTES: [u32; 2] = [0, 30];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ScheduleMode {
    Recurring,
    OneShot,
}

impl ScheduleMode {
    pub fn from_recurring_flag(recurring: bool) -> Self {
        if recurring {
            Self::Recurring
        } else {
            Self::OneShot
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PersistenceMode {
    Session,
    Durable,
}

impl PersistenceMode {
    pub fn from_durable_flag(durable: bool) -> Self {
        if durable {
            Self::Durable
        } else {
            Self::Session
        }
    }

    pub fn is_durable(self) -> bool {
        matches!(self, Self::Durable)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTaskRecord {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub mode: ScheduleMode,
    pub persistence: PersistenceMode,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(
        rename = "last_fired",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_fired_at: Option<i64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub jitter_offset_minutes: u64,
}

impl fmt::Display for ScheduledTaskRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let age_hours = ((Local::now().timestamp() - self.created_at) as f64 / 3600.0).max(0.0);
        write!(
            f,
            "{}  {}  [{}/{}] ({age_hours:.1}h old): {}",
            self.id,
            self.cron,
            self.mode,
            self.persistence,
            truncate_chars(&self.prompt, 60)
        )
    }
}

#[derive(Debug, Clone)]
pub struct CronNotification {
    pub task_id: String,
    pub prompt: String,
}

impl fmt::Display for CronNotification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[Scheduled task {}]: {}", self.task_id, self.prompt)
    }
}

#[derive(Debug, Clone)]
pub struct MissedTask {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub missed_at: String,
}

#[derive(Debug)]
struct CronScheduler {
    claude_dir: PathBuf,
    tasks: Mutex<HashMap<String, ScheduledTaskRecord>>,
    notification_tx: mpsc::Sender<CronNotification>,
    notification_rx: Mutex<mpsc::Receiver<CronNotification>>,
    stop_requested: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
    next_id: AtomicU64,
}

#[derive(Clone, Debug)]
pub struct SharedCronScheduler {
    inner: Arc<CronScheduler>,
}

impl SharedCronScheduler {
    pub fn new(workdir: impl AsRef<Path>) -> Result<Self> {
        let claude_dir = workdir.as_ref().join(".claude");
        fs::create_dir_all(&claude_dir)
            .with_context(|| format!("failed to create {}", claude_dir.display()))?;
        let (notification_tx, notification_rx) = mpsc::channel();

        Ok(Self {
            inner: Arc::new(CronScheduler {
                claude_dir,
                tasks: Mutex::new(HashMap::new()),
                notification_tx,
                notification_rx: Mutex::new(notification_rx),
                stop_requested: AtomicBool::new(false),
                worker: Mutex::new(None),
                next_id: AtomicU64::new(Local::now().timestamp_millis().max(0) as u64),
            }),
        })
    }

    pub fn start(&self) -> Result<usize> {
        self.load_durable()?;
        self.inner.stop_requested.store(false, Ordering::SeqCst);

        let mut worker = self
            .inner
            .worker
            .lock()
            .map_err(|_| anyhow::anyhow!("cron scheduler lock poisoned"))?;
        if worker.is_some() {
            return self.task_count();
        }

        let scheduler = self.clone();
        *worker = Some(tokio::spawn(async move {
            scheduler.check_loop().await;
        }));
        self.task_count()
    }

    pub async fn stop(&self) {
        self.inner.stop_requested.store(true, Ordering::SeqCst);
        let handle = {
            let Ok(mut worker) = self.inner.worker.lock() else {
                return;
            };
            worker.take()
        };

        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }

    pub fn create(
        &self,
        cron_expr: &str,
        prompt: &str,
        recurring: bool,
        durable: bool,
    ) -> Result<String> {
        validate_schedule(cron_expr)?;

        let id = format!("{:08x}", self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let mode = ScheduleMode::from_recurring_flag(recurring);
        let persistence = PersistenceMode::from_durable_flag(durable);
        let task = ScheduledTaskRecord {
            id: id.clone(),
            cron: cron_expr.trim().to_string(),
            prompt: prompt.to_string(),
            mode,
            persistence,
            created_at: Local::now().timestamp(),
            last_fired_at: None,
            jitter_offset_minutes: if matches!(mode, ScheduleMode::Recurring) {
                compute_jitter(cron_expr)
            } else {
                0
            },
        };

        self.with_tasks(|tasks| {
            tasks.insert(task.id.clone(), task.clone());
            Ok(())
        })?;

        if persistence.is_durable() {
            self.save_durable()?;
        }

        Ok(format!(
            "Created task {} ({}, {}): cron={}",
            task.id, task.mode, task.persistence, task.cron
        ))
    }

    pub fn delete(&self, task_id: &str) -> Result<String> {
        let removed = self.with_tasks(|tasks| Ok(tasks.remove(task_id)))?;
        match removed {
            Some(task) => {
                if task.persistence.is_durable() {
                    self.save_durable()?;
                }
                Ok(format!("Deleted task {}", task_id))
            }
            None => Ok(format!("Task {} not found", task_id)),
        }
    }

    pub fn list_tasks(&self) -> Result<String> {
        let mut tasks = self.with_tasks(|tasks| Ok(tasks.values().cloned().collect::<Vec<_>>()))?;
        if tasks.is_empty() {
            return Ok("No scheduled tasks.".to_string());
        }
        tasks.sort_by_key(|task| task.created_at);
        Ok(tasks
            .into_iter()
            .map(|task| task.to_string())
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn drain_notifications(&self) -> Vec<CronNotification> {
        let Ok(receiver) = self.inner.notification_rx.lock() else {
            return Vec::new();
        };

        let mut notifications = Vec::new();
        while let Ok(note) = receiver.try_recv() {
            notifications.push(note);
        }
        notifications
    }

    pub fn enqueue_test_notification(&self) {
        let _ = self.inner.notification_tx.send(CronNotification {
            task_id: "test-0000".to_string(),
            prompt: "This is a test notification.".to_string(),
        });
    }

    pub fn detect_missed_tasks(&self) -> Result<Vec<MissedTask>> {
        let now = Local::now();
        let tasks = self.with_tasks(|tasks| Ok(tasks.values().cloned().collect::<Vec<_>>()))?;
        let mut missed = Vec::new();

        for task in tasks {
            let Some(last_fired_at) = task.last_fired_at else {
                continue;
            };
            let Ok(schedule) = parse_schedule(&task.cron) else {
                continue;
            };
            let Some(last_dt) =
                DateTime::from_timestamp(last_fired_at, 0).map(|dt| dt.with_timezone(&Local))
            else {
                continue;
            };

            let mut check = last_dt + ChronoDuration::minutes(1);
            let cap = std::cmp::min(now, last_dt + ChronoDuration::hours(24));
            while check <= cap {
                if schedule_matches(&schedule, check) {
                    missed.push(MissedTask {
                        id: task.id.clone(),
                        cron: task.cron.clone(),
                        prompt: task.prompt.clone(),
                        missed_at: check.to_rfc3339(),
                    });
                    break;
                }
                check += ChronoDuration::minutes(1);
            }
        }

        Ok(missed)
    }

    async fn check_loop(&self) {
        let mut lock_file: Option<File> = None;
        let mut last_check_minute: Option<i64> = None;
        let mut ticker = interval(Duration::from_secs(CHECK_INTERVAL_SECS));

        loop {
            ticker.tick().await;
            if self.inner.stop_requested.load(Ordering::SeqCst) {
                break;
            }

            if lock_file.is_none() {
                match self.try_acquire_cron_lock() {
                    Ok(Some(file)) => lock_file = Some(file),
                    Ok(None) => {}
                    Err(error) => eprintln!("[Cron] lock error: {}", error),
                }
            }

            if lock_file.is_some() {
                let now = Local::now();
                let current_minute = (now.hour() as i64) * 60 + now.minute() as i64;
                if last_check_minute != Some(current_minute) {
                    last_check_minute = Some(current_minute);
                    if let Err(error) = self.check_tasks(now) {
                        eprintln!("[Cron] scheduler error: {}", error);
                    }
                }
            }
        }

        if let Some(file) = lock_file.take() {
            let _ = file.unlock();
        }
    }

    fn check_tasks(&self, now: DateTime<Local>) -> Result<()> {
        let now_ts = now.timestamp();
        let mut notifications = Vec::new();
        let mut expired = Vec::new();
        let mut one_shots = Vec::new();
        let mut should_save = false;

        let durable_snapshot = self.with_tasks(|tasks| {
            for task in tasks.values_mut() {
                let age_days = (now_ts - task.created_at) / 86_400;
                if matches!(task.mode, ScheduleMode::Recurring) && age_days > AUTO_EXPIRY_DAYS {
                    expired.push(task.id.clone());
                    continue;
                }

                let Ok(schedule) = parse_schedule(&task.cron) else {
                    continue;
                };
                let check_time = now - ChronoDuration::minutes(task.jitter_offset_minutes as i64);
                if schedule_matches(&schedule, check_time) {
                    notifications.push(CronNotification {
                        task_id: task.id.clone(),
                        prompt: task.prompt.clone(),
                    });
                    task.last_fired_at = Some(now_ts);
                    if task.persistence.is_durable() {
                        should_save = true;
                    }
                    if matches!(task.mode, ScheduleMode::OneShot) {
                        one_shots.push(task.id.clone());
                    }
                }
            }

            if !expired.is_empty() || !one_shots.is_empty() {
                should_save = true;
                tasks.retain(|id, _| !expired.contains(id) && !one_shots.contains(id));
            }

            Ok(tasks
                .values()
                .filter(|task| task.persistence.is_durable())
                .cloned()
                .collect::<Vec<_>>())
        })?;

        for note in notifications {
            println!("[Cron] Fired: {}", note.task_id);
            let _ = self.inner.notification_tx.send(note);
        }
        for task_id in expired {
            println!(
                "[Cron] Auto-expired: {} (older than {} days)",
                task_id, AUTO_EXPIRY_DAYS
            );
        }
        for task_id in one_shots {
            println!("[Cron] One-shot completed and removed: {}", task_id);
        }

        if should_save {
            self.save_durable_snapshot(&durable_snapshot)?;
        }
        Ok(())
    }

    fn load_durable(&self) -> Result<()> {
        let path = self.scheduled_tasks_file();
        if !path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let tasks: Vec<ScheduledTaskRecord> =
            serde_json::from_str(&content).context("failed to parse durable scheduled tasks")?;

        self.with_tasks(|store| {
            store.retain(|_, task| !task.persistence.is_durable());
            for task in tasks {
                store.insert(task.id.clone(), task);
            }
            Ok(())
        })?;
        Ok(())
    }

    fn save_durable(&self) -> Result<()> {
        let snapshot = self.with_tasks(|tasks| {
            Ok(tasks
                .values()
                .filter(|task| task.persistence.is_durable())
                .cloned()
                .collect::<Vec<_>>())
        })?;
        self.save_durable_snapshot(&snapshot)
    }

    fn save_durable_snapshot(&self, tasks: &[ScheduledTaskRecord]) -> Result<()> {
        fs::create_dir_all(&self.inner.claude_dir)?;
        let content =
            serde_json::to_string_pretty(tasks).context("failed to serialize scheduled tasks")?;
        fs::write(self.scheduled_tasks_file(), format!("{content}\n"))
            .context("failed to persist durable scheduled tasks")?;
        Ok(())
    }

    fn task_count(&self) -> Result<usize> {
        self.with_tasks(|tasks| Ok(tasks.len()))
    }

    fn scheduled_tasks_file(&self) -> PathBuf {
        self.inner.claude_dir.join("scheduled_tasks.json")
    }

    fn lock_file(&self) -> PathBuf {
        self.inner.claude_dir.join("cron.lock")
    }

    fn try_acquire_cron_lock(&self) -> Result<Option<File>> {
        let path = self.lock_file();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("failed to open lock file {}", path.display()))?;

        match file.try_lock_exclusive() {
            Ok(true) => Ok(Some(file)),
            Ok(false) => Ok(None),
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error)
                .with_context(|| format!("failed to acquire cron lock {}", path.display())),
        }
    }

    fn with_tasks<T>(
        &self,
        callback: impl FnOnce(&mut HashMap<String, ScheduledTaskRecord>) -> Result<T>,
    ) -> Result<T> {
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .map_err(|_| anyhow::anyhow!("cron scheduler lock poisoned"))?;
        callback(&mut tasks)
    }
}

fn validate_schedule(expr: &str) -> Result<()> {
    let _ = parse_schedule(expr)?;
    Ok(())
}

fn parse_schedule(expr: &str) -> Result<Schedule> {
    Schedule::from_str(expr.trim())
        .with_context(|| format!("invalid cron expression: {}", expr.trim()))
}

fn schedule_matches(schedule: &Schedule, dt: DateTime<Local>) -> bool {
    let Some(at_minute) = dt
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .map(|value| value.with_timezone(&Utc))
    else {
        return false;
    };

    let previous = at_minute - ChronoDuration::minutes(1);
    schedule.after(&previous).next() == Some(at_minute)
}

fn compute_jitter(expr: &str) -> u64 {
    let Some(minute_field) = expr.split_whitespace().next() else {
        return 0;
    };
    let Ok(minute) = minute_field.parse::<u32>() else {
        return 0;
    };
    if !JITTER_TARGET_MINUTES.contains(&minute) {
        return 0;
    }

    let mut hasher = DefaultHasher::new();
    expr.trim().hash(&mut hasher);
    (hasher.finish() % JITTER_OFFSET_MAX_MINUTES) + 1
}

fn truncate_chars(text: &str, limit: usize) -> String {
    let truncated = text.chars().take(limit).collect::<String>();
    if text.chars().count() > limit {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}
