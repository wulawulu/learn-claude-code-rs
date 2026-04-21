use super::*;

impl SharedTeammateManager {
    pub(super) fn lock_config(&self) -> Result<MutexGuard<'_, TeamConfig>> {
        self.inner
            .config
            .lock()
            .map_err(|_| anyhow::anyhow!("team config lock poisoned"))
    }

    pub(super) fn lock_handles(&self) -> Result<MutexGuard<'_, HashMap<String, JoinHandle<()>>>> {
        self.inner
            .handles
            .lock()
            .map_err(|_| anyhow::anyhow!("team handles lock poisoned"))
    }

    pub(super) fn save_config_locked(&self, config: &TeamConfig) -> Result<()> {
        let _ = &self.inner.team_dir;
        fs::write(
            &self.inner.config_path,
            serde_json::to_string_pretty(config)?,
        )
        .with_context(|| format!("failed to write {}", self.inner.config_path.display()))
    }
}
