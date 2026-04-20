use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use anthropic_ai_sdk::types::message::{Message, Role};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};
use tokio::task::JoinHandle;

use crate::{LoopState, get_llm_client, tool::teammate_tools};

pub const TEAM_DIR_NAME: &str = ".team";
const INBOX_DIR_NAME: &str = "inbox";
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxMessage {
    #[serde(rename = "type")]
    pub message_type: MessageType,
    #[serde(rename = "from")]
    pub from_name: String,
    pub content: String,
    pub timestamp: f64,
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
        let message = InboxMessage {
            message_type,
            from_name: sender.to_string(),
            content: content.to_string(),
            timestamp: unix_timestamp(),
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

struct TeammateManagerInner {
    team_dir: PathBuf,
    config_path: PathBuf,
    config: Mutex<TeamConfig>,
    handles: Mutex<HashMap<String, JoinHandle<()>>>,
    message_bus: SharedMessageBus,
}

#[derive(Clone)]
pub struct SharedTeammateManager {
    inner: Arc<TeammateManagerInner>,
}

impl SharedTeammateManager {
    pub fn new(team_dir: impl AsRef<Path>, message_bus: SharedMessageBus) -> Result<Self> {
        let team_dir = team_dir.as_ref().to_path_buf();
        fs::create_dir_all(&team_dir)
            .with_context(|| format!("failed to create team dir {}", team_dir.display()))?;

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
                message_bus,
            }),
        })
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

        let manager = self.clone();
        let name = name.to_string();
        let role = role.to_string();
        let prompt = prompt.to_string();
        let task_name = name.clone();
        let task_role = role.clone();

        let handle = tokio::spawn(async move {
            if let Err(error) = manager
                .teammate_loop(task_name.clone(), task_role, prompt)
                .await
            {
                eprintln!("[teammate:{task_name}] {error:#}");
                let _ = manager.finish_member(&task_name, false);
            }
        });

        if let Ok(mut handles) = self.inner.handles.lock() {
            handles.insert(name.clone(), handle);
        }

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

    async fn teammate_loop(&self, name: String, role: String, prompt: String) -> Result<()> {
        let client = get_llm_client()?;
        let tools = teammate_tools(self.inner.message_bus.clone(), name.clone());

        let workdir = std::env::current_dir()?;
        let system_prompt = format!(
            "You are '{name}', role: {role}, at {}. Use send_message to communicate. Complete your task.",
            workdir.display()
        );

        let mut state = LoopState::new(
            client,
            tools,
            self.inner.message_bus.clone(),
            name.clone(),
            system_prompt,
            50,
        );
        state.context.push(Message::new_text(Role::User, prompt));
        let result = state.agent_loop().await;
        self.finish_member(&name, result.is_ok())?;
        result
    }

    fn finish_member(&self, name: &str, success: bool) -> Result<()> {
        let mut config = self.lock_config()?;

        if let Some(member) = config.members.iter_mut().find(|member| member.name == name) {
            if success || member.status != TeammateStatus::Shutdown {
                member.status = TeammateStatus::Idle;
            }
            self.save_config_locked(&config)?;
        }

        let mut handles = self.lock_handles()?;
        handles.remove(name);

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

    fn save_config_locked(&self, config: &TeamConfig) -> Result<()> {
        let _ = &self.inner.team_dir;
        fs::write(
            &self.inner.config_path,
            serde_json::to_string_pretty(config)?,
        )
        .with_context(|| format!("failed to write {}", self.inner.config_path.display()))
    }
}

fn unix_timestamp() -> f64 {
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
