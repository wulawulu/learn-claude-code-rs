pub mod memory;
pub mod prompt;
pub mod skill;
pub mod tool;
pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;
use serde_json::Value;

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient as _, MessageContent,
        MessageError, RequiredMessageParams, Role, StopReason,
    },
};
use anyhow::{Context, Result};
use chrono::Utc;

use crate::{
    memory::{MEMORY_GUIDANCE, MemoryManager},
    prompt::SystemPrompt,
    skill::SkillRegistry,
    tool::Tool,
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
    pub skill_registry: Arc<SkillRegistry>,
    pub memory_manager: Arc<Mutex<MemoryManager>>,
}

impl LoopState {
    pub fn new(
        client: AnthropicClient,
        tools: HashMap<String, Box<dyn Tool>>,
        skill_registry: Arc<SkillRegistry>,
        memory_manager: Arc<Mutex<MemoryManager>>,
    ) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
            skill_registry,
            memory_manager,
        }
    }

    pub async fn agent_loop(&mut self) -> Result<()> {
        let system = self.build_system_prompt()?;
        loop {
            let request = CreateMessageParams::new(RequiredMessageParams {
                model: MODEL.to_string(),
                messages: self.context.clone(),
                max_tokens: 8000,
            })
            .with_system(&system)
            .with_tools(self.tools.values().map(|tool| tool.tool_spec()).collect());

            let response = self.client.create_message(Some(&request)).await?;

            self.context.push(Message::new_blocks(
                Role::Assistant,
                response.content.clone(),
            ));

            if let Some(stop_reason) = response.stop_reason
                && !matches!(stop_reason, StopReason::ToolUse)
            {
                return Ok(());
            }

            self.execute_tool_call(&response.content).await?;
        }
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

    fn build_system_prompt(&self) -> Result<String> {
        let workdir = std::env::current_dir()?;
        let prompt = SystemPrompt::builder()
            .role(format!(
                "You are a coding agent operating in {}.",
                workdir.display()
            ))
            .guidelines([
                "Try to understand how to complete the task well before completing it.",
            ])
            .constraints([
                "Think step by step",
                "Think before you act; respond with your thoughts before calling tools",
                "Do not make up any assumptions, use tools to get the information you need",
                "Use the provided tools to interact with the system and accomplish the task",
                "If you are stuck, or otherwise cannot complete the task, respond with your thoughts and stop",
                "If the task is completed, or otherwise cannot continue, like requiring user feedback, stop.",
            ])
            .skills_available(self.skill_registry.describe_available())
            .memory(self.load_memory_prompt()?)
            .claude_md(self.load_claude_md_prompt(&workdir))
            .dynamic_context(self.load_dynamic_context(&workdir))
            .memory_guidance(MEMORY_GUIDANCE.trim())
            .build()?;

        prompt
            .to_prompt()
            .render()
            .context("failed to render system prompt")
    }

    fn load_memory_prompt(&self) -> Result<String> {
        self.memory_manager
            .lock()
            .map_err(|_| anyhow::anyhow!("memory manager lock poisoned"))
            .map(|manager| manager.load_memory_prompt())
    }

    fn load_claude_md_prompt(&self, workdir: &Path) -> String {
        let mut sources = Vec::new();

        let user_claude = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| home.join(".claude").join("CLAUDE.md"));
        if let Some(path) = user_claude
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            sources.push((
                "user global (~/.claude/CLAUDE.md)".to_string(),
                content.trim().to_string(),
            ));
        }

        let project_claude = workdir.join("CLAUDE.md");
        if let Ok(content) = std::fs::read_to_string(&project_claude) {
            sources.push((
                "project root (CLAUDE.md)".to_string(),
                content.trim().to_string(),
            ));
        }

        if let Ok(cwd) = std::env::current_dir()
            && cwd != workdir
        {
            let subdir_claude = cwd.join("CLAUDE.md");
            if let Ok(content) = std::fs::read_to_string(&subdir_claude) {
                sources.push((
                    format!("subdir ({}/CLAUDE.md)", cwd.display()),
                    content.trim().to_string(),
                ));
            }
        }

        if sources.is_empty() {
            return String::new();
        }

        let mut lines = vec!["# CLAUDE.md instructions".to_string(), String::new()];
        for (label, content) in sources {
            lines.push(format!("## From {}", label));
            lines.push(String::new());
            lines.push(content);
            lines.push(String::new());
        }
        lines.join("\n").trim().to_string()
    }

    fn load_dynamic_context(&self, workdir: &Path) -> String {
        let lines = [
            "# Dynamic context".to_string(),
            format!("Current date: {}", Utc::now().date_naive()),
            format!("Working directory: {}", workdir.display()),
            format!("Model: {}", MODEL),
            format!("Platform: {}", std::env::consts::OS),
        ];
        lines.join("\n")
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
