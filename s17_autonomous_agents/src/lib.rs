pub mod task;
pub mod team;
pub mod tool;
pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;
use serde_json::Value;
use tokio::time::sleep;

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient as _, MessageContent,
        MessageError, RequiredMessageParams, Role, StopReason,
    },
};
use anyhow::{Context, Result};

use crate::team::{InboxMessage, MessageType, SharedTeammateManager};
use crate::tool::Tool;
use crate::{
    task::TaskRecord,
    team::{IDLE_TIMEOUT_SECS, POLL_INTERVAL_SECS, TeammateStatus},
};

pub const MODEL: &str = "deepseek-chat";

pub fn get_llm_client() -> anyhow::Result<AnthropicClient> {
    dotenvy::dotenv().ok();

    let anthropic_api_key =
        std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY is not set")?;
    let anthropic_base_url =
        std::env::var("ANTHROPIC_BASE_URL").context("ANTHROPIC_BASE_URL is not set")?;
    let client = AnthropicClientBuilder::new(anthropic_api_key, "")
        .with_api_base_url(anthropic_base_url)
        .build::<MessageError>()
        .context("can't create client")?;
    Ok(client)
}

pub struct LoopState {
    pub client: AnthropicClient,
    pub context: Vec<Message>,
    pub tools: HashMap<String, Box<dyn Tool>>,
    pub manager: SharedTeammateManager,
    pub inbox_owner: String,
    pub system_prompt: String,
    pub max_rounds: usize,
    pub stop_signal: Option<Arc<AtomicBool>>,
    identity: Option<LoopIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentLoopExit {
    Idle,
    Shutdown,
}

#[derive(Debug, Clone)]
struct LoopIdentity {
    name: String,
    role: String,
    team_name: String,
}

impl LoopState {
    pub fn new(
        client: AnthropicClient,
        tools: HashMap<String, Box<dyn Tool>>,
        manager: SharedTeammateManager,
        inbox_owner: impl Into<String>,
        system_prompt: impl Into<String>,
        max_rounds: usize,
    ) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
            manager,
            inbox_owner: inbox_owner.into(),
            system_prompt: system_prompt.into(),
            max_rounds,
            stop_signal: None,
            identity: None,
        }
    }

    pub fn with_stop_signal(mut self, stop_signal: Arc<AtomicBool>) -> Self {
        self.stop_signal = Some(stop_signal);
        self
    }

    pub fn with_identity(
        mut self,
        name: impl Into<String>,
        role: impl Into<String>,
        team_name: impl Into<String>,
    ) -> Self {
        self.identity = Some(LoopIdentity {
            name: name.into(),
            role: role.into(),
            team_name: team_name.into(),
        });
        self
    }

    pub async fn agent_loop(&mut self) -> Result<AgentLoopExit> {
        for _ in 0..self.max_rounds {
            if self.should_stop() {
                return Ok(AgentLoopExit::Shutdown);
            }

            let inbox = self.read_inbox()?;
            if self.contains_shutdown_request(&inbox) {
                self.request_stop();
                return Ok(AgentLoopExit::Shutdown);
            }
            self.inject_inbox_messages(&inbox)?;

            let request = CreateMessageParams::new(RequiredMessageParams {
                model: MODEL.to_string(),
                messages: self.context.clone(),
                max_tokens: 8000,
            })
            .with_system(&self.system_prompt)
            .with_tools(self.tools.values().map(|tool| tool.tool_spec()).collect());

            let response = self.client.create_message(Some(&request)).await?;

            self.context.push(Message::new_blocks(
                Role::Assistant,
                response.content.clone(),
            ));

            if let Some(stop_reason) = response.stop_reason
                && !matches!(stop_reason, StopReason::ToolUse)
            {
                return Ok(AgentLoopExit::Idle);
            }

            self.execute_tool_call(&response.content).await?;

            let idle_requested = response
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolUse { name, .. } if name == "idle"));

            if idle_requested {
                return Ok(AgentLoopExit::Idle);
            }
        }

        Ok(AgentLoopExit::Idle)
    }

    pub(crate) async fn run_autonomous_teammate_loop(&mut self) -> Result<()> {
        let mut status = TeammateStatus::Working;
        self.manager
            .set_status_if_changed(&self.inbox_owner, status)?;

        loop {
            status = match status {
                TeammateStatus::Working => self.run_work_phase().await?,
                TeammateStatus::Idle => self.run_idle_polling_phase().await?,
                TeammateStatus::Shutdown => return Ok(()),
            };

            if status == TeammateStatus::Shutdown {
                return Ok(());
            }

            self.manager
                .set_status_if_changed(&self.inbox_owner, status)?;
        }
    }

    async fn run_work_phase(&mut self) -> Result<TeammateStatus> {
        match self.agent_loop().await? {
            AgentLoopExit::Idle => Ok(TeammateStatus::Idle),
            AgentLoopExit::Shutdown => Ok(TeammateStatus::Shutdown),
        }
    }

    async fn run_idle_polling_phase(&mut self) -> Result<TeammateStatus> {
        let polls = IDLE_TIMEOUT_SECS / POLL_INTERVAL_SECS.max(1);
        let role = self
            .identity
            .as_ref()
            .map(|identity| identity.role.as_str())
            .context("autonomous teammate loop requires identity")?;

        for _ in 0..polls {
            sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;

            if self.should_stop() {
                return Ok(TeammateStatus::Shutdown);
            }

            let inbox = self.read_inbox()?;
            if self.contains_shutdown_request(&inbox) {
                self.request_stop();
                return Ok(TeammateStatus::Shutdown);
            }
            if !inbox.is_empty() {
                self.resume_from_idle_with_inbox(&inbox)?;
                return Ok(TeammateStatus::Working);
            }

            if let Some((task, claim_result)) =
                self.manager.auto_claim_task(&self.inbox_owner, role)?
            {
                self.resume_from_idle_with_auto_claim(&task, &claim_result);
                return Ok(TeammateStatus::Working);
            }
        }

        Ok(TeammateStatus::Shutdown)
    }

    pub(crate) fn should_stop(&self) -> bool {
        self.stop_signal
            .as_ref()
            .is_some_and(|signal| signal.load(Ordering::Relaxed))
    }

    pub(crate) fn request_stop(&self) {
        if let Some(stop_signal) = &self.stop_signal {
            stop_signal.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn read_inbox(&self) -> Result<Vec<InboxMessage>> {
        self.manager.read_inbox(&self.inbox_owner)
    }

    pub(crate) fn contains_shutdown_request(&self, inbox: &[InboxMessage]) -> bool {
        self.stop_signal.is_some()
            && inbox
                .iter()
                .any(|message| matches!(message.message_type, MessageType::ShutdownRequest))
    }

    fn inject_inbox_messages(&mut self, inbox: &[InboxMessage]) -> Result<()> {
        if inbox.is_empty() {
            return Ok(());
        }

        let content = format!("<inbox>{}</inbox>", serde_json::to_string_pretty(&inbox)?);
        self.context.push(Message::new_text(Role::User, content));
        Ok(())
    }

    pub fn resume_from_idle_with_inbox(&mut self, inbox: &[InboxMessage]) -> Result<()> {
        self.ensure_identity_context();
        self.inject_inbox_messages(inbox)
    }

    pub fn resume_from_idle_with_auto_claim(&mut self, task: &TaskRecord, claim_result: &str) {
        self.ensure_identity_context();
        let description = task.description.as_deref().unwrap_or_default();
        self.context.push(Message::new_text(
            Role::User,
            format!(
                "<auto-claimed>Task #{}: {}\n{description}</auto-claimed>",
                task.id, task.subject
            ),
        ));
        self.context.push(Message::new_text(
            Role::Assistant,
            format!("{claim_result}. Working on it."),
        ));
    }

    fn ensure_identity_context(&mut self) {
        let Some(identity) = self.identity.as_ref() else {
            return;
        };
        if self.has_identity_context() {
            return;
        }

        self.context.insert(0, make_identity_block(identity));
        self.context.insert(
            1,
            Message::new_text(
                Role::Assistant,
                format!("I am {}. Continuing.", identity.name),
            ),
        );
    }

    fn has_identity_context(&self) -> bool {
        self.context
            .first()
            .map(|message| extract_text(&message.content).contains("<identity>"))
            .unwrap_or(false)
    }

    pub async fn execute_tool_call(&mut self, content: &[ContentBlock]) -> anyhow::Result<()> {
        let mut result = Vec::new();
        for block in content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                let output = self.execute(name, input).await;
                result.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: output,
                });
            }
        }
        self.context.push(Message::new_blocks(Role::User, result));
        Ok(())
    }

    async fn execute(&mut self, name: &str, input: &Value) -> String {
        let Some(tool) = self.tools.get_mut(name) else {
            return format!("Unknown tool: {}", name);
        };

        match tool.invoke(input).await {
            Ok(output) => {
                println!(
                    "Command:{}\n arg:{}\n output:\n{}\n",
                    name,
                    input,
                    output.chars().take(200).collect::<String>()
                );
                output
            }
            Err(e) => {
                println!("Error invoking tool {}: {}", name, e);
                format!("Error invoking tool {}: {}", name, e)
            }
        }
    }
}

fn make_identity_block(identity: &LoopIdentity) -> Message {
    Message::new_text(
        Role::User,
        format!(
            "<identity>You are '{}', role: {}, team: {}. Continue your work.</identity>",
            identity.name, identity.role, identity.team_name
        ),
    )
}

pub fn extract_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { content } => content.clone(),
        MessageContent::Blocks { content } => content
            .iter()
            .filter_map(|block| {
                if let ContentBlock::Text { text } = block {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}
