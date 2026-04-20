use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TeammateStatus {
    Idle,
    Working,
    Shutdown,
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

impl SharedTeammateManager {
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
        self.spawn_teammate_task(name.to_string(), role.to_string(), prompt.to_string())?;

        Ok(format!("Spawned '{}' (role: {})", name, role))
    }

    pub(super) fn spawn_teammate_task(
        &self,
        name: String,
        role: String,
        prompt: String,
    ) -> Result<()> {
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
            }
            if let Err(error) = manager.cleanup_finished_teammate(&task_name) {
                eprintln!("[teammate:{task_name}] finalize failed: {error:#}");
            }
        });

        self.lock_handles()?.insert(name, handle);
        Ok(())
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

    pub(super) async fn teammate_loop(
        &self,
        name: String,
        role: String,
        prompt: String,
        stop_signal: Arc<AtomicBool>,
    ) -> Result<()> {
        let client = get_llm_client()?;
        let tools = teammate_tools(teammate_tools_input(
            self.clone(),
            self.inner.tasks.clone(),
            name.clone(),
            role.clone(),
        ));

        let workdir = std::env::current_dir()?;
        let team_name = self.lock_config()?.team_name.clone();
        let system_prompt = format!(
            "You are '{name}', role: {role}, team: {team_name}, at {}. Use idle when you have no more immediate work. During idle you will poll for inbox messages and auto-claim ready tasks that match your role. Submit plans via plan_approval before major work. Respond to shutdown_request with shutdown_response.",
            workdir.display(),
        );

        let mut state =
            LoopState::new(client, tools, self.clone(), name.clone(), system_prompt, 50)
                .with_stop_signal(stop_signal.clone())
                .with_identity(name.clone(), role.clone(), team_name.clone());
        state.context.push(Message::new_text(Role::User, prompt));
        state.run_autonomous_teammate_loop().await
    }

    pub fn auto_claim_task(&self, name: &str, role: &str) -> Result<Option<(TaskRecord, String)>> {
        let Some(task) = self
            .inner
            .tasks
            .scan_unclaimed(Some(role))?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };

        let claim_result = self
            .inner
            .tasks
            .claim(task.id, name, Some(role), ClaimSource::Auto)?;
        if claim_result.starts_with("Error:") {
            return Ok(None);
        }

        Ok(Some((self.inner.tasks.get_record(task.id)?, claim_result)))
    }

    pub fn set_status_if_changed(&self, name: &str, status: TeammateStatus) -> Result<()> {
        let mut config = self.lock_config()?;
        if let Some(member) = config.members.iter_mut().find(|member| member.name == name) {
            if member.status == status {
                return Ok(());
            }
            member.status = status;
            self.save_config_locked(&config)?;
        }
        Ok(())
    }
}
