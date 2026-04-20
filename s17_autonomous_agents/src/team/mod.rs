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
    task::{ClaimSource, SharedTaskManager, TaskRecord},
    team::{message::SharedMessageBus, requests::SharedRequestStore},
    tool::{teammate_tools, teammate_tools_input},
};

mod cleanup;
mod message;
mod requests;
mod storage;
mod teammate;

pub use message::{InboxMessage, MessageType};
pub use teammate::{TeamConfig, TeamMember, TeammateStatus};

pub const TEAM_DIR_NAME: &str = ".team";
const INBOX_DIR_NAME: &str = "inbox";
const REQUESTS_DIR_NAME: &str = "requests";
const DEFAULT_TEAM_NAME: &str = "default";
pub const POLL_INTERVAL_SECS: u64 = 5;
pub const IDLE_TIMEOUT_SECS: u64 = 60;

struct TeammateManagerInner {
    team_dir: PathBuf,
    config_path: PathBuf,
    config: Mutex<TeamConfig>,
    handles: Mutex<HashMap<String, JoinHandle<()>>>,
    stop_signals: Mutex<HashMap<String, Arc<AtomicBool>>>,
    message_bus: SharedMessageBus,
    request_store: SharedRequestStore,
    tasks: SharedTaskManager,
}

#[derive(Clone)]
pub struct SharedTeammateManager {
    inner: Arc<TeammateManagerInner>,
}

impl SharedTeammateManager {
    pub fn new(team_dir: impl AsRef<Path>, tasks: SharedTaskManager) -> Result<Self> {
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
                tasks,
            }),
        })
    }
}

pub fn unix_timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or_default()
}
