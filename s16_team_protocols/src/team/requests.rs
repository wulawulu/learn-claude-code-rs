use super::*;

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

impl SharedTeammateManager {
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
}
