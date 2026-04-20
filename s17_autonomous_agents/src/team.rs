use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anthropic_ai_sdk::types::message::{Message, Role};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use strum_macros::{Display, EnumString};
use tokio::task::JoinHandle;

use crate::{
    LoopState, get_llm_client,
    tool::{teammate_tools, teammate_tools_input},
};

pub const TEAM_DIR_NAME: &str = ".team";
const INBOX_DIR_NAME: &str = "inbox";
const REQUESTS_DIR_NAME: &str = "requests";
const DEFAULT_TEAM_NAME: &str = "default";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MessageType {
    Message,
    Broadcast,
    ShutdownRequest,
    ShutdownResponse,
    PlanApproval,
    PlanApprovalResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TeammateStatus {
    Idle,
    Working,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RequestKind {
    Shutdown,
    PlanApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxMessage {
    #[serde(rename = "type")]
    pub message_type: MessageType,
    #[serde(rename = "from")]
    pub from_name: String,
    pub content: String,
    pub timestamp: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<HashMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub name: String,
    pub role: String,
    pub status: TeammateStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    pub team_name: String,
    pub members: Vec<TeamMember>,
}

impl Default for TeamConfig {
    fn default() -> Self {
        Self {
            team_name: DEFAULT_TEAM_NAME.to_string(),
            members: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolResponse {
    pub approve: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRecord {
    pub request_id: String,
    pub kind: RequestKind,
    #[serde(rename = "from")]
    pub from_name: String,
    #[serde(rename = "to")]
    pub to_name: String,
    pub status: RequestStatus,
    pub created_at: f64,
    pub updated_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<ProtocolResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<f64>,
}

impl RequestRecord {
    pub fn pending_shutdown(request_id: String, teammate: impl Into<String>) -> Self {
        let now = unix_timestamp();
        Self {
            request_id,
            kind: RequestKind::Shutdown,
            from_name: "lead".to_string(),
            to_name: teammate.into(),
            status: RequestStatus::Pending,
            created_at: now,
            updated_at: now,
            plan: None,
            response: None,
            feedback: None,
            reviewed_by: None,
            resolved_by: None,
            resolved_at: None,
        }
    }

    pub fn pending_plan_approval(
        request_id: String,
        from_name: impl Into<String>,
        plan: impl Into<String>,
    ) -> Self {
        let now = unix_timestamp();
        Self {
            request_id,
            kind: RequestKind::PlanApproval,
            from_name: from_name.into(),
            to_name: "lead".to_string(),
            status: RequestStatus::Pending,
            created_at: now,
            updated_at: now,
            plan: Some(plan.into()),
            response: None,
            feedback: None,
            reviewed_by: None,
            resolved_by: None,
            resolved_at: None,
        }
    }

    pub fn resolve_shutdown(&mut self, sender: impl Into<String>, approve: bool, reason: &str) {
        self.status = approval_status(approve);
        self.resolved_by = Some(sender.into());
        self.resolved_at = Some(unix_timestamp());
        self.response = Some(ProtocolResponse {
            approve,
            message: reason.to_string(),
        });
    }

    pub fn review_plan(&mut self, approve: bool, feedback: &str) {
        self.status = approval_status(approve);
        self.reviewed_by = Some("lead".to_string());
        self.feedback = (!feedback.is_empty()).then(|| feedback.to_string());
        self.resolved_at = Some(unix_timestamp());
    }
}

struct MessageBusInner {
    inbox_dir: PathBuf,
    io_guard: Mutex<()>,
}

#[derive(Clone)]
pub struct SharedMessageBus {
    inner: Arc<MessageBusInner>,
}

impl SharedMessageBus {
    pub fn new(team_dir: impl AsRef<Path>) -> Result<Self> {
        let inbox_dir = team_dir.as_ref().join(INBOX_DIR_NAME);
        fs::create_dir_all(&inbox_dir)
            .with_context(|| format!("failed to create inbox dir {}", inbox_dir.display()))?;

        Ok(Self {
            inner: Arc::new(MessageBusInner {
                inbox_dir,
                io_guard: Mutex::new(()),
            }),
        })
    }

    pub fn send(
        &self,
        sender: &str,
        to: &str,
        content: &str,
        message_type: MessageType,
    ) -> Result<String> {
        self.send_with_extra(sender, to, content, message_type, None)
    }

    pub fn send_with_extra(
        &self,
        sender: &str,
        to: &str,
        content: &str,
        message_type: MessageType,
        extra: Option<HashMap<String, Value>>,
    ) -> Result<String> {
        let message = InboxMessage {
            message_type,
            from_name: sender.to_string(),
            content: content.to_string(),
            timestamp: unix_timestamp(),
            extra,
        };

        let path = self.inbox_path(to);
        let _guard = self
            .inner
            .io_guard
            .lock()
            .map_err(|_| anyhow::anyhow!("message bus lock poisoned"))?;

        let mut existing = String::new();
        if path.exists() {
            existing = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
        }

        existing.push_str(&serde_json::to_string(&message)?);
        existing.push('\n');

        fs::write(&path, existing)
            .with_context(|| format!("failed to write {}", path.display()))?;

        Ok(format!("Sent {} to {}", message.message_type, to))
    }

    pub fn read_inbox(&self, name: &str) -> Result<Vec<InboxMessage>> {
        let path = self.inbox_path(name);
        let _guard = self
            .inner
            .io_guard
            .lock()
            .map_err(|_| anyhow::anyhow!("message bus lock poisoned"))?;

        if !path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        let messages = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str::<InboxMessage>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .with_context(|| format!("failed to parse {}", path.display()))?;

        fs::write(&path, "").with_context(|| format!("failed to clear {}", path.display()))?;
        Ok(messages)
    }

    pub fn broadcast(&self, sender: &str, content: &str, teammates: &[String]) -> Result<String> {
        let mut count = 0usize;
        for name in teammates {
            if name == sender {
                continue;
            }
            self.send(sender, name, content, MessageType::Broadcast)?;
            count += 1;
        }
        Ok(format!("Broadcast to {} teammates", count))
    }

    pub fn register_mailbox(&self, name: &str) {
        let path = self.inbox_path(name);
        if !path.exists() {
            let _ = fs::write(path, "");
        }
    }

    fn inbox_path(&self, name: &str) -> PathBuf {
        self.inner.inbox_dir.join(format!("{name}.jsonl"))
    }
}

struct RequestStoreInner {
    dir: PathBuf,
    io_guard: Mutex<()>,
    next_id: AtomicU64,
}

#[derive(Clone)]
pub struct SharedRequestStore {
    inner: Arc<RequestStoreInner>,
}

impl SharedRequestStore {
    pub fn new(team_dir: impl AsRef<Path>) -> Result<Self> {
        let dir = team_dir.as_ref().join(REQUESTS_DIR_NAME);
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create requests dir {}", dir.display()))?;

        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default();

        Ok(Self {
            inner: Arc::new(RequestStoreInner {
                dir,
                io_guard: Mutex::new(()),
                next_id: AtomicU64::new(seed),
            }),
        })
    }

    pub fn next_request_id(&self) -> String {
        format!("{:08x}", self.inner.next_id.fetch_add(1, Ordering::Relaxed))
    }

    pub fn create(&self, record: RequestRecord) -> Result<RequestRecord> {
        let _guard = self.lock_io()?;
        fs::write(
            self.path(&record.request_id),
            serde_json::to_string_pretty(&record)?,
        )?;
        Ok(record)
    }

    pub fn get(&self, request_id: &str) -> Result<Option<RequestRecord>> {
        let _guard = self.lock_io()?;
        self.read_record(request_id)
    }

    pub fn update(
        &self,
        request_id: &str,
        updater: impl FnOnce(&mut RequestRecord),
    ) -> Result<Option<RequestRecord>> {
        let _guard = self.lock_io()?;
        let Some(mut record) = self.read_record(request_id)? else {
            return Ok(None);
        };
        updater(&mut record);
        record.updated_at = unix_timestamp();
        fs::write(
            self.path(request_id),
            serde_json::to_string_pretty(&record)?,
        )?;
        Ok(Some(record))
    }

    pub fn status_json(&self, request_id: &str) -> Result<String> {
        let record = self
            .get(request_id)?
            .map(serde_json::to_value)
            .transpose()?
            .unwrap_or_else(|| json!({ "error": "not found" }));
        Ok(serde_json::to_string_pretty(&record)?)
    }

    fn read_record(&self, request_id: &str) -> Result<Option<RequestRecord>> {
        let path = self.path(request_id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    fn path(&self, request_id: &str) -> PathBuf {
        self.inner.dir.join(format!("{request_id}.json"))
    }

    fn lock_io(&self) -> Result<MutexGuard<'_, ()>> {
        self.inner
            .io_guard
            .lock()
            .map_err(|_| anyhow::anyhow!("request store lock poisoned"))
    }
}

struct TeammateManagerInner {
    team_dir: PathBuf,
    config_path: PathBuf,
    config: Mutex<TeamConfig>,
    handles: Mutex<HashMap<String, JoinHandle<()>>>,
    stop_signals: Mutex<HashMap<String, Arc<AtomicBool>>>,
    message_bus: SharedMessageBus,
    request_store: SharedRequestStore,
}

#[derive(Clone)]
pub struct SharedTeammateManager {
    inner: Arc<TeammateManagerInner>,
}

impl SharedTeammateManager {
    pub fn new(team_dir: impl AsRef<Path>) -> Result<Self> {
        let team_dir = team_dir.as_ref().to_path_buf();
        fs::create_dir_all(&team_dir)
            .with_context(|| format!("failed to create team dir {}", team_dir.display()))?;
        let message_bus = SharedMessageBus::new(&team_dir)?;
        let request_store = SharedRequestStore::new(&team_dir)?;

        let config_path = team_dir.join("config.json");
        let config = if config_path.exists() {
            let raw = fs::read_to_string(&config_path)
                .with_context(|| format!("failed to read {}", config_path.display()))?;
            serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse {}", config_path.display()))?
        } else {
            TeamConfig::default()
        };

        Ok(Self {
            inner: Arc::new(TeammateManagerInner {
                team_dir,
                config_path,
                config: Mutex::new(config),
                handles: Mutex::new(HashMap::new()),
                stop_signals: Mutex::new(HashMap::new()),
                message_bus,
                request_store,
            }),
        })
    }

    pub fn register_mailbox(&self, name: &str) {
        self.inner.message_bus.register_mailbox(name);
    }

    pub fn spawn(&self, name: &str, role: &str, prompt: &str) -> Result<String> {
        {
            let mut config = self.lock_config()?;

            if let Some(member) = config.members.iter_mut().find(|member| member.name == name) {
                if !matches!(
                    member.status,
                    TeammateStatus::Idle | TeammateStatus::Shutdown
                ) {
                    return Ok(format!("Error: '{}' is currently {}", name, member.status));
                }
                member.role = role.to_string();
                member.status = TeammateStatus::Working;
            } else {
                config.members.push(TeamMember {
                    name: name.to_string(),
                    role: role.to_string(),
                    status: TeammateStatus::Working,
                });
            }

            self.save_config_locked(&config)?;
        }

        self.inner.message_bus.register_mailbox(name);
        self.spawn_member_task(name.to_string(), role.to_string(), prompt.to_string())?;

        Ok(format!("Spawned '{}' (role: {})", name, role))
    }

    pub fn list_all(&self) -> Result<String> {
        let config = self.lock_config()?;

        if config.members.is_empty() {
            return Ok("No teammates.".to_string());
        }

        let mut lines = vec![format!("Team: {}", config.team_name)];
        for member in &config.members {
            lines.push(format!(
                "  {} ({}): {}",
                member.name, member.role, member.status
            ));
        }

        Ok(lines.join("\n"))
    }

    pub fn member_names(&self) -> Result<Vec<String>> {
        let config = self.lock_config()?;
        Ok(config
            .members
            .iter()
            .map(|member| member.name.clone())
            .collect())
    }

    pub fn send_message(
        &self,
        sender: &str,
        to: &str,
        content: &str,
        message_type: MessageType,
    ) -> Result<String> {
        self.inner
            .message_bus
            .send(sender, to, content, message_type)
    }

    pub fn read_inbox_json(&self, owner: &str) -> Result<String> {
        let inbox = self.inner.message_bus.read_inbox(owner)?;
        Ok(serde_json::to_string_pretty(&inbox)?)
    }

    pub fn read_inbox(&self, owner: &str) -> Result<Vec<InboxMessage>> {
        self.inner.message_bus.read_inbox(owner)
    }

    pub fn broadcast(&self, sender: &str, content: &str) -> Result<String> {
        let teammates = self.member_names()?;
        self.inner
            .message_bus
            .broadcast(sender, content, &teammates)
    }

    pub fn create_shutdown_request(&self, teammate: &str) -> Result<String> {
        let request_id = self.inner.request_store.next_request_id();
        self.inner
            .request_store
            .create(RequestRecord::pending_shutdown(
                request_id.clone(),
                teammate.to_string(),
            ))?;

        self.inner.message_bus.send_with_extra(
            "lead",
            teammate,
            "Please shut down gracefully.",
            MessageType::ShutdownRequest,
            Some(HashMap::from([(
                "request_id".to_string(),
                Value::String(request_id.clone()),
            )])),
        )?;

        Ok(format!(
            "Shutdown request {} sent to '{}' (status: pending)",
            request_id, teammate
        ))
    }

    pub fn respond_shutdown(
        &self,
        sender: &str,
        request_id: &str,
        approve: bool,
        reason: &str,
    ) -> Result<String> {
        let Some(_) = self.inner.request_store.update(request_id, |record| {
            record.resolve_shutdown(sender.to_string(), approve, reason);
        })?
        else {
            return Ok(format!("Error: Unknown shutdown request {}", request_id));
        };

        self.inner.message_bus.send_with_extra(
            sender,
            "lead",
            reason,
            MessageType::ShutdownResponse,
            Some(HashMap::from([
                (
                    "request_id".to_string(),
                    Value::String(request_id.to_string()),
                ),
                ("approve".to_string(), Value::Bool(approve)),
            ])),
        )?;

        if approve && let Some(stop_signal) = self.lock_stop_signals()?.get(sender).cloned() {
            stop_signal.store(true, Ordering::Relaxed);
        }

        Ok(format!(
            "Shutdown {}",
            if approve { "approved" } else { "rejected" }
        ))
    }

    pub fn review_plan(&self, request_id: &str, approve: bool, feedback: &str) -> Result<String> {
        let Some(record) = self.inner.request_store.get(request_id)? else {
            return Ok(format!("Error: Unknown plan request_id '{}'", request_id));
        };

        self.inner
            .request_store
            .update(request_id, |record| record.review_plan(approve, feedback))?;

        self.inner.message_bus.send_with_extra(
            "lead",
            &record.from_name,
            feedback,
            MessageType::PlanApprovalResponse,
            Some(HashMap::from([
                (
                    "request_id".to_string(),
                    Value::String(request_id.to_string()),
                ),
                ("approve".to_string(), Value::Bool(approve)),
                ("feedback".to_string(), Value::String(feedback.to_string())),
            ])),
        )?;

        Ok(format!(
            "Plan {} for '{}'",
            if approve { "approved" } else { "rejected" },
            record.from_name
        ))
    }

    pub fn shutdown_status(&self, request_id: &str) -> Result<String> {
        self.inner.request_store.status_json(request_id)
    }

    pub fn submit_plan(&self, sender: &str, plan: &str) -> Result<String> {
        let request_id = self.inner.request_store.next_request_id();
        self.inner
            .request_store
            .create(RequestRecord::pending_plan_approval(
                request_id.clone(),
                sender.to_string(),
                plan.to_string(),
            ))?;

        self.inner.message_bus.send_with_extra(
            sender,
            "lead",
            plan,
            MessageType::PlanApproval,
            Some(HashMap::from([
                ("request_id".to_string(), Value::String(request_id.clone())),
                ("plan".to_string(), Value::String(plan.to_string())),
            ])),
        )?;

        Ok(format!(
            "Plan submitted (request_id={request_id}). Waiting for lead approval."
        ))
    }

    async fn teammate_loop(
        &self,
        name: String,
        role: String,
        prompt: String,
        stop_signal: Arc<AtomicBool>,
    ) -> Result<()> {
        let client = get_llm_client()?;
        let tools = teammate_tools(teammate_tools_input(self.clone(), name.clone()));

        let workdir = std::env::current_dir()?;
        let system_prompt = format!(
            "You are '{name}', role: {role}, at {}. Submit plans via plan_approval before major work. Respond to shutdown_request with shutdown_response.",
            workdir.display()
        );

        let mut state =
            LoopState::new(client, tools, self.clone(), name.clone(), system_prompt, 50)
                .with_stop_signal(stop_signal.clone());
        state.context.push(Message::new_text(Role::User, prompt));
        let result = state.agent_loop().await;
        self.finish_member(&name, stop_signal.load(Ordering::Relaxed))?;
        result
    }

    fn finish_member(&self, name: &str, should_shutdown: bool) -> Result<()> {
        let mut config = self.lock_config()?;

        if let Some(member) = config.members.iter_mut().find(|member| member.name == name) {
            member.status = if should_shutdown {
                TeammateStatus::Shutdown
            } else {
                TeammateStatus::Idle
            };
            self.save_config_locked(&config)?;
        }

        let mut handles = self.lock_handles()?;
        handles.remove(name);
        self.lock_stop_signals()?.remove(name);

        Ok(())
    }

    fn spawn_member_task(&self, name: String, role: String, prompt: String) -> Result<()> {
        let manager = self.clone();
        let stop_signal = Arc::new(AtomicBool::new(false));

        self.lock_stop_signals()?
            .insert(name.clone(), stop_signal.clone());

        let task_name = name.clone();
        let handle = tokio::spawn(async move {
            if let Err(error) = manager
                .teammate_loop(task_name.clone(), role, prompt, stop_signal)
                .await
            {
                eprintln!("[teammate:{task_name}] {error:#}");
                let _ = manager.finish_member(&task_name, false);
            }
        });

        self.lock_handles()?.insert(name, handle);
        Ok(())
    }

    fn lock_config(&self) -> Result<MutexGuard<'_, TeamConfig>> {
        self.inner
            .config
            .lock()
            .map_err(|_| anyhow::anyhow!("team config lock poisoned"))
    }

    fn lock_handles(&self) -> Result<MutexGuard<'_, HashMap<String, JoinHandle<()>>>> {
        self.inner
            .handles
            .lock()
            .map_err(|_| anyhow::anyhow!("team handles lock poisoned"))
    }

    fn lock_stop_signals(&self) -> Result<MutexGuard<'_, HashMap<String, Arc<AtomicBool>>>> {
        self.inner
            .stop_signals
            .lock()
            .map_err(|_| anyhow::anyhow!("team stop_signals lock poisoned"))
    }

    fn save_config_locked(&self, config: &TeamConfig) -> Result<()> {
        let _ = &self.inner.team_dir;
        fs::write(
            &self.inner.config_path,
            serde_json::to_string_pretty(config)?,
        )
        .with_context(|| format!("failed to write {}", self.inner.config_path.display()))
    }
}

pub fn unix_timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or_default()
}

impl fmt::Display for InboxMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} from {}: {}",
            self.message_type, self.from_name, self.content
        )
    }
}

fn approval_status(approve: bool) -> RequestStatus {
    if approve {
        RequestStatus::Approved
    } else {
        RequestStatus::Rejected
    }
}
