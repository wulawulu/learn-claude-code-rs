use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::store::{CollectionStore, Store, StoreRoot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateRecord {
    pub name: String,
    pub role: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamConfig {
    #[serde(default)]
    pub teammates: Vec<TeammateRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxMessage {
    pub from: String,
    pub to: String,
    pub body: String,
    pub kind: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct TeammateManager {
    config: Store<TeamConfig>,
    inboxes: CollectionStore<InboxMessage>,
}

#[derive(Clone, Debug)]
pub struct SharedTeammateManager {
    inner: Arc<Mutex<TeammateManager>>,
}

impl TeammateManager {
    pub fn new(root: &StoreRoot) -> Result<Self> {
        let manager = Self {
            config: root.file("team/config.json")?,
            inboxes: root.collection("team/inbox")?,
        };
        if !manager.config.exists() {
            manager.config.write(&TeamConfig::default())?;
        }
        Ok(manager)
    }

    pub fn spawn_teammate(&mut self, name: String, role: String) -> Result<String> {
        let mut config = self.config.read().unwrap_or_default();
        if config
            .teammates
            .iter()
            .any(|teammate| teammate.name == name)
        {
            anyhow::bail!("teammate {name} already exists");
        }
        let record = TeammateRecord {
            name,
            role,
            status: "idle".to_string(),
        };
        config.teammates.push(record.clone());
        self.config.write(&config)?;
        serde_json::to_string_pretty(&record).context("failed to serialize teammate")
    }

    pub fn list_teammates(&self) -> Result<String> {
        let config = self.config.read().unwrap_or_default();
        if config.teammates.is_empty() {
            return Ok("No teammates.".to_string());
        }
        Ok(config
            .teammates
            .into_iter()
            .map(|teammate| format!("{} [{}] {}", teammate.name, teammate.role, teammate.status))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn send_message(&mut self, from: String, to: String, body: String) -> Result<String> {
        let message = InboxMessage {
            from,
            to: to.clone(),
            body,
            kind: "message".to_string(),
            created_at: Utc::now(),
        };
        self.inboxes.append(&to, &message)?;
        Ok(format!("sent message to {to}"))
    }

    pub fn broadcast(&mut self, from: String, body: String) -> Result<String> {
        let config = self.config.read().unwrap_or_default();
        for teammate in &config.teammates {
            self.send_message(from.clone(), teammate.name.clone(), body.clone())?;
        }
        Ok(format!("broadcast to {} teammates", config.teammates.len()))
    }

    pub fn read_inbox(&self, owner: &str) -> Result<String> {
        let messages = self.inboxes.read_all_from(owner)?;
        if messages.is_empty() {
            return Ok("Inbox is empty.".to_string());
        }
        serde_json::to_string_pretty(&messages).context("failed to serialize inbox")
    }

    pub fn protocol_request(
        &mut self,
        from: String,
        to: String,
        kind: String,
        body: String,
    ) -> Result<String> {
        let message = InboxMessage {
            from,
            to: to.clone(),
            body,
            kind,
            created_at: Utc::now(),
        };
        self.inboxes.append(&to, &message)?;
        Ok(format!("sent protocol request to {to}"))
    }
}

impl SharedTeammateManager {
    pub fn new(manager: TeammateManager) -> Self {
        Self {
            inner: Arc::new(Mutex::new(manager)),
        }
    }

    pub fn with_manager<T>(&self, f: impl FnOnce(&mut TeammateManager) -> Result<T>) -> Result<T> {
        let mut manager = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("teammate manager lock poisoned"))?;
        f(&mut manager)
    }

    pub fn spawn_teammate(&self, name: String, role: String) -> Result<String> {
        self.with_manager(|manager| manager.spawn_teammate(name, role))
    }

    pub fn list_teammates(&self) -> Result<String> {
        self.with_manager(|manager| manager.list_teammates())
    }

    pub fn send_message(&self, from: String, to: String, body: String) -> Result<String> {
        self.with_manager(|manager| manager.send_message(from, to, body))
    }

    pub fn broadcast(&self, from: String, body: String) -> Result<String> {
        self.with_manager(|manager| manager.broadcast(from, body))
    }

    pub fn read_inbox(&self, owner: &str) -> Result<String> {
        self.with_manager(|manager| manager.read_inbox(owner))
    }

    pub fn protocol_request(
        &self,
        from: String,
        to: String,
        kind: String,
        body: String,
    ) -> Result<String> {
        self.with_manager(|manager| manager.protocol_request(from, to, kind, body))
    }
}

impl std::ops::Deref for SharedTeammateManager {
    type Target = Arc<Mutex<TeammateManager>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
