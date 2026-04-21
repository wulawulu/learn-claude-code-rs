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
}
