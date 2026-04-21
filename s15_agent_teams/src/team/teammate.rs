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
}
