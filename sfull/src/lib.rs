pub mod background;
pub mod compact;
pub mod cron;
pub mod hook;
pub mod mcp;
pub mod memory;
pub mod permission;
pub mod prompt;
pub mod recovery;
pub mod skill;
pub mod store;
pub mod task;
pub mod team;
pub mod tool;
pub mod worktree;

pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient, MessageContent, MessageError,
        RequiredMessageParams, Role, StopReason,
    },
};
use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;

use crate::compact::{
    CompactState, compacted_context, estimate_context_size, micro_compact, persist_large_output,
    write_transcript,
};
use crate::hook::{
    Hook, HookControl, HookTypes, PostToolUseFn, PreToolUseFn, SessionStartFn, ToolResult, ToolUse,
};
use crate::mcp::MCPToolRouter;
use crate::memory::MEMORY_GUIDANCE;
use crate::permission::{PermissionBehavior, PermissionManager};
use crate::prompt::SystemPrompt;
use crate::recovery::{
    CONTINUATION_MESSAGE, MAX_RECOVERY_ATTEMPTS, RecoveryState, backoff_delay,
    is_prompt_too_long_error, is_transient_transport_error,
};
use crate::tool::{ToolContext, ToolRouter};

pub const MODEL: &str = "deepseek-v4-pro";
const CONTEXT_LIMIT: usize = 50_000;

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

pub struct AgentRuntime {
    pub client: AnthropicClient,
    pub context: Vec<Message>,
    pub compact_state: CompactState,
    pub recovery_state: RecoveryState,
    pub permission_manager: PermissionManager,
}

pub enum AgentSystemPrompt {
    Dynamic,
    Static(String),
}

pub struct Agent {
    pub runtime: AgentRuntime,
    pub tool_context: ToolContext,
    pub tools: ToolRouter,
    pub mcp_router: MCPToolRouter,
    pub hooks: Vec<Hook>,
    pub system_prompt: AgentSystemPrompt,
}

impl Agent {
    pub fn new(
        client: AnthropicClient,
        tool_context: ToolContext,
        tools: ToolRouter,
        mcp_router: MCPToolRouter,
        permission_manager: PermissionManager,
        system_prompt: AgentSystemPrompt,
    ) -> Self {
        Self {
            runtime: AgentRuntime {
                client,
                context: Vec::new(),
                compact_state: CompactState::default(),
                recovery_state: RecoveryState::default(),
                permission_manager,
            },
            tool_context,
            tools,
            mcp_router,
            hooks: Vec::new(),
            system_prompt,
        }
    }

    pub async fn agent_loop(&mut self) -> Result<()> {
        self.runtime.recovery_state = RecoveryState::default();
        let system = self.build_system_prompt()?;
        loop {
            micro_compact(&mut self.runtime.context);
            if estimate_context_size(&self.runtime.context) > CONTEXT_LIMIT {
                println!("[auto compact]");
                self.compact_history(None).await?;
            }

            let request = CreateMessageParams::new(RequiredMessageParams {
                model: MODEL.to_string(),
                messages: self.runtime.context.clone(),
                max_tokens: 8000,
            })
            .with_system(&system)
            .with_tools(self.all_tool_specs());

            let response = match self.runtime.client.create_message(Some(&request)).await {
                Ok(response) => {
                    self.runtime.recovery_state.transport_attempts = 0;
                    response
                }
                Err(error) => {
                    let error_text = error.to_string().to_lowercase();
                    if is_prompt_too_long_error(&error_text)
                        && self.runtime.recovery_state.compact_attempts < MAX_RECOVERY_ATTEMPTS
                    {
                        self.runtime.recovery_state.compact_attempts += 1;
                        println!(
                            "[Recovery] compact ({}/{}): context too large",
                            self.runtime.recovery_state.compact_attempts, MAX_RECOVERY_ATTEMPTS
                        );
                        self.compact_history(None).await?;
                        continue;
                    }

                    if is_transient_transport_error(&error_text)
                        && self.runtime.recovery_state.transport_attempts < MAX_RECOVERY_ATTEMPTS
                    {
                        let delay = backoff_delay(self.runtime.recovery_state.transport_attempts);
                        self.runtime.recovery_state.transport_attempts += 1;
                        println!(
                            "[Recovery] backoff ({}/{}): retrying in {:.1}s",
                            self.runtime.recovery_state.transport_attempts,
                            MAX_RECOVERY_ATTEMPTS,
                            delay.as_secs_f64()
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }

                    return Err(error).context("message request failed");
                }
            };

            self.runtime.context.push(Message::new_blocks(
                Role::Assistant,
                response.content.clone(),
            ));

            if matches!(response.stop_reason, Some(StopReason::MaxTokens))
                && self.runtime.recovery_state.continuation_attempts < MAX_RECOVERY_ATTEMPTS
            {
                self.runtime.recovery_state.continuation_attempts += 1;
                println!(
                    "[Recovery] continue ({}/{}): output truncated",
                    self.runtime.recovery_state.continuation_attempts, MAX_RECOVERY_ATTEMPTS
                );
                self.runtime
                    .context
                    .push(Message::new_text(Role::User, CONTINUATION_MESSAGE));
                continue;
            }
            self.runtime.recovery_state.continuation_attempts = 0;

            if let Some(stop_reason) = response.stop_reason
                && !matches!(stop_reason, StopReason::ToolUse)
            {
                return Ok(());
            }

            let (tool_result, manual_compact) = self.execute_tool_call(&response.content).await?;

            self.runtime
                .context
                .push(Message::new_blocks(Role::User, tool_result));

            if let Some(focus) = manual_compact {
                println!("[manual compact]");
                self.compact_history(Some(focus.as_str())).await?;
            }
        }
    }

    pub async fn execute_tool_call(
        &mut self,
        content: &[ContentBlock],
    ) -> Result<(Vec<ContentBlock>, Option<String>)> {
        let mut result = Vec::new();
        let mut manual_compact = None;
        for block in content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                let mut tool_use = ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                };
                let output = match invoke_hooks!(PreToolUse, self, &mut tool_use) {
                    Ok(HookControl::Continue) => {
                        let decision = self
                            .runtime
                            .permission_manager
                            .check(&tool_use.name, &tool_use.input);
                        match decision.behavior {
                            PermissionBehavior::Allow => {}
                            PermissionBehavior::Deny => {
                                return Ok((
                                    vec![ContentBlock::ToolResult {
                                        tool_use_id: id.clone(),
                                        content: format!("Permission denied: {}", decision.reason),
                                    }],
                                    manual_compact,
                                ));
                            }
                            PermissionBehavior::Ask => {
                                if !self
                                    .runtime
                                    .permission_manager
                                    .ask_user(&tool_use.name, &tool_use.input)?
                                {
                                    return Ok((
                                        vec![ContentBlock::ToolResult {
                                            tool_use_id: id.clone(),
                                            content: format!(
                                                "Permission denied by user for {}",
                                                tool_use.name
                                            ),
                                        }],
                                        manual_compact,
                                    ));
                                }
                            }
                        }
                        let mut result = ToolResult {
                            tool_use_id: tool_use.id.clone(),
                            content: self
                                .execute(&tool_use.id, &tool_use.name, &tool_use.input)
                                .await,
                        };
                        match invoke_hooks!(PostToolUse, self, &tool_use, &mut result) {
                            Ok(HookControl::Continue) => result.content,
                            Ok(HookControl::Block(reason)) => {
                                format!("Tool blocked by PostToolUse hook: {reason}")
                            }
                            Err(error) => format!("PostToolUse hook failed: {error}"),
                        }
                    }
                    Ok(HookControl::Block(reason)) => {
                        format!("Tool blocked by PreToolUse hook: {reason}")
                    }
                    Err(error) => format!("PreToolUse hook failed: {error}"),
                };
                result.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: output,
                });
                if tool_use.name == "read_file"
                    && let Some(path) = tool_use.input.get("path").and_then(|value| value.as_str())
                {
                    self.remember_recent_file(path);
                }
                if tool_use.name == "compact" {
                    manual_compact = tool_use
                        .input
                        .get("focus")
                        .and_then(|value| value.as_str())
                        .map(ToOwned::to_owned)
                        .or_else(|| Some(String::new()));
                }
            }
        }
        Ok((result, manual_compact))
    }

    pub fn session_start(&mut self, hook: impl SessionStartFn + 'static) {
        self.hooks.push(Hook::SessionStart(Box::new(hook)));
    }

    pub fn post_tool(&mut self, hook: impl PostToolUseFn + 'static) {
        self.hooks.push(Hook::PostToolUse(Box::new(hook)));
    }

    pub fn pre_tool(&mut self, hook: impl PreToolUseFn + 'static) {
        self.hooks.push(Hook::PreToolUse(Box::new(hook)));
    }

    pub fn hooks_by_type(&self, hook_type: HookTypes) -> Vec<&Hook> {
        self.hooks
            .iter()
            .filter(|hook| hook_type == (*hook).into())
            .collect()
    }

    pub fn all_tool_specs(&self) -> Vec<ToolSpec> {
        self.tools
            .tool_specs()
            .into_iter()
            .chain(self.mcp_router.all_tools())
            .collect()
    }

    async fn execute(
        &mut self,
        tool_use_id: &str,
        name: &str,
        input: &serde_json::Value,
    ) -> String {
        if MCPToolRouter::is_mcp_tool(name) {
            return match self.mcp_router.call(name, input.clone()).await {
                Ok(output) => {
                    println!(
                        "MCP tool:{}\n arg:{}\n output:\n{}\n",
                        name,
                        input,
                        output.chars().take(200).collect::<String>()
                    );
                    output
                }
                Err(e) => {
                    println!("Error invoking MCP tool {}: {}", name, e);
                    format!("Error invoking MCP tool {}: {}", name, e)
                }
            };
        }

        match self
            .tools
            .call(&self.tool_context, name, input.clone())
            .await
        {
            Ok(output) => {
                let output = if name == "bash" {
                    match persist_large_output(tool_use_id, &output) {
                        Ok(compacted) => compacted,
                        Err(e) => format!("Error persisting large output: {}", e),
                    }
                } else {
                    output
                };
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

    pub async fn compact_history(&mut self, focus: Option<&str>) -> Result<()> {
        let transcript_path = write_transcript(&self.runtime.context)?;
        println!("[transcript saved: {}]", transcript_path.display());

        let conversation_text = serde_json::to_string(&self.runtime.context)
            .context("failed to serialize conversation for summarization")?;
        let truncated = conversation_text.chars().take(80_000).collect::<String>();
        let mut prompt = format!(
            "Summarize this coding-agent conversation so work can continue.\n\
Preserve:\n\
1. The current goal\n\
2. Important findings and decisions\n\
3. Files read or changed\n\
4. Remaining work\n\
5. User constraints and preferences\n\
Be compact but concrete.\n\n\
{truncated}"
        );
        if let Some(focus) = focus.filter(|value| !value.trim().is_empty()) {
            prompt.push_str(&format!("\n\nFocus to preserve next: {focus}"));
        }
        if !self.runtime.compact_state.recent_files.is_empty() {
            let recent = self
                .runtime
                .compact_state
                .recent_files
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n");
            prompt.push_str(&format!("\n\nRecent files to reopen if needed:\n{recent}"));
        }

        let request = CreateMessageParams::new(RequiredMessageParams {
            model: MODEL.to_string(),
            messages: vec![Message::new_text(Role::User, prompt)],
            max_tokens: 2000,
        });
        let response = self.runtime.client.create_message(Some(&request)).await?;
        let summary = response
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        self.runtime.compact_state.has_compacted = true;
        self.runtime.compact_state.last_summary = Some(summary.clone());
        self.runtime.context = compacted_context(summary);
        Ok(())
    }

    fn remember_recent_file(&mut self, path: &str) {
        self.runtime
            .compact_state
            .recent_files
            .retain(|existing| existing != path);
        self.runtime
            .compact_state
            .recent_files
            .push(path.to_string());
        if self.runtime.compact_state.recent_files.len() > 5 {
            let overflow = self.runtime.compact_state.recent_files.len() - 5;
            self.runtime.compact_state.recent_files.drain(0..overflow);
        }
    }

    fn build_system_prompt(&self) -> Result<String> {
        if let AgentSystemPrompt::Static(system_prompt) = &self.system_prompt {
            return Ok(system_prompt.clone());
        }

        let workdir = &self.tool_context.work_dir;
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
            .skills_available(self.tool_context.skill_registry.describe_available())
            .memory(self.load_memory_prompt()?)
            .claude_md(load_claude_md_prompt(workdir))
            .dynamic_context(load_dynamic_context(workdir))
            .memory_guidance(MEMORY_GUIDANCE.trim())
            .build()?;

        prompt
            .to_prompt()
            .render()
            .context("failed to render system prompt")
    }

    fn load_memory_prompt(&self) -> Result<String> {
        self.tool_context
            .memory_manager
            .lock()
            .map_err(|_| anyhow::anyhow!("memory manager lock poisoned"))
            .map(|manager| manager.load_memory_prompt())
    }
}

pub type LoopState = Agent;

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

fn load_dynamic_context(workdir: &Path) -> String {
    let lines = [
        "# Dynamic context".to_string(),
        format!("Current date: {}", Utc::now().date_naive()),
        format!("Working directory: {}", workdir.display()),
        format!("Model: {}", MODEL),
        format!("Platform: {}", std::env::consts::OS),
    ];
    lines.join("\n")
}

fn load_claude_md_prompt(workdir: &Path) -> String {
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
