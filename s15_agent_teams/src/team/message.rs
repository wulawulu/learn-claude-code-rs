use super::*;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxMessage {
    #[serde(rename = "type")]
    pub message_type: MessageType,
    #[serde(rename = "from")]
    pub from_name: String,
    pub content: String,
    pub timestamp: f64,
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
