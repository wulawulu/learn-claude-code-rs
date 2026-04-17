pub mod tool;

pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;
use serde_json::Value;

use std::{
    collections::HashMap,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient as _, MessageContent,
        MessageError, RequiredMessageParams, Role, StopReason,
    },
};
use anyhow::{Context, Result};

use crate::tool::Tool;

pub const MODEL: &str = "deepseek-chat";

const MAX_RECOVERY_ATTEMPTS: u32 = 3;
const BACKOFF_BASE_DELAY_SECS: f64 = 1.0;
const BACKOFF_MAX_DELAY_SECS: f64 = 30.0;
const CONTEXT_THRESHOLD_CHARS: usize = 50_000;
const CONTINUATION_MESSAGE: &str = "Output limit hit. Continue directly from where you stopped. \
No recap, no repetition. Pick up mid-sentence if needed.";

#[derive(Debug, Default)]
struct RecoveryState {
    continuation_attempts: u32,
    compact_attempts: u32,
    transport_attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopControl {
    Continue,
    Stop,
}

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
}

impl LoopState {
    pub fn new(client: AnthropicClient, tools: HashMap<String, Box<dyn Tool>>) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
        }
    }

    pub async fn agent_loop(&mut self) -> Result<()> {
        let system = format!(
            "You are a coding agent at {}. Use tools to solve tasks.",
            std::env::current_dir()?.display(),
        );
        // Recovery budget is scoped to one query / one agent_loop call.
        let mut recovery = RecoveryState::default();

        loop {
            let request = CreateMessageParams::new(RequiredMessageParams {
                model: MODEL.to_string(),
                messages: self.context.clone(),
                max_tokens: 8000,
            })
            .with_system(&system)
            .with_tools(self.tools.values().map(|tool| tool.tool_spec()).collect());

            let Some(response) = (match self.client.create_message(Some(&request)).await {
                Ok(response) => {
                    recovery.transport_attempts = 0;
                    Some(response)
                }
                Err(error) => {
                    match self
                        .handle_request_error_recovery(error, &mut recovery)
                        .await?
                    {
                        LoopControl::Continue => {
                            continue;
                        }
                        LoopControl::Stop => None,
                    }
                }
            }) else {
                return Ok(());
            };

            self.context.push(Message::new_blocks(
                Role::Assistant,
                response.content.clone(),
            ));

            if matches!(response.stop_reason, Some(StopReason::MaxTokens))
                && self.handle_max_tokens_recovery(&mut recovery)
            {
                continue;
            }

            recovery.continuation_attempts = 0;

            if let Some(stop_reason) = response.stop_reason
                && !matches!(stop_reason, StopReason::ToolUse)
            {
                return Ok(());
            }

            self.execute_tool_call(&response.content).await?;
            self.maybe_auto_compact().await?;
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

    pub async fn compact_history(&mut self) -> anyhow::Result<()> {
        let conversation_text = serde_json::to_string(&self.context)
            .context("failed to serialize conversation for summarization")?;
        let truncated = conversation_text.chars().take(80000).collect::<String>();

        let prompt = format!(
            "Summarize this conversation for continuity. Include:\n\
            1) Task overview and success criteria\n\
            2) Current state: completed work, files touched\n\
            3) Key decisions and failed approaches\n\
            4) Remaining next steps\n\
            Be concise but preserve critical details.\n\n\
            {}",
            truncated
        );

        let request = CreateMessageParams::new(RequiredMessageParams {
            model: MODEL.to_string(),
            messages: vec![Message::new_text(Role::User, prompt)],
            max_tokens: 4000,
        });

        let response = self.client.create_message(Some(&request)).await?;
        let summary = response
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        self.context = vec![Message::new_text(
            Role::User,
            format!(
                "This session continues from a previous conversation that was compacted. \n\
                Summary of prior context:\n\n{summary}\n\n\
                Continue from where we left off without re-asking the user."
            ),
        )];
        Ok(())
    }

    async fn handle_request_error_recovery(
        &mut self,
        error: MessageError,
        recovery: &mut RecoveryState,
    ) -> Result<LoopControl> {
        let error_text = error.to_string().to_lowercase();
        if is_prompt_too_long_error(&error_text) {
            if recovery.compact_attempts >= MAX_RECOVERY_ATTEMPTS {
                println!(
                    "[Error] compact recovery exhausted after {} attempts: {}",
                    MAX_RECOVERY_ATTEMPTS, error
                );
                return Ok(LoopControl::Stop);
            }

            recovery.compact_attempts += 1;
            println!(
                "[Recovery] compact ({}/{}): context too large",
                recovery.compact_attempts, MAX_RECOVERY_ATTEMPTS
            );
            if let Err(compact_error) = self.compact_history().await {
                println!("[Error] compact recovery failed: {}", compact_error);
                return Ok(LoopControl::Stop);
            }
            return Ok(LoopControl::Continue);
        }

        if is_transient_transport_error(&error_text) {
            if recovery.transport_attempts >= MAX_RECOVERY_ATTEMPTS {
                println!(
                    "[Error] transport recovery exhausted after {} attempts: {}",
                    MAX_RECOVERY_ATTEMPTS, error
                );
                return Ok(LoopControl::Stop);
            }

            let delay = backoff_delay(recovery.transport_attempts);
            recovery.transport_attempts += 1;
            println!(
                "[Recovery] backoff ({}/{}): transient transport failure. Retrying in {:.1}s",
                recovery.transport_attempts,
                MAX_RECOVERY_ATTEMPTS,
                delay.as_secs_f64()
            );
            thread::sleep(delay);
            return Ok(LoopControl::Continue);
        }

        println!("[Error] API call failed: {}", error);
        Ok(LoopControl::Stop)
    }

    fn handle_max_tokens_recovery(&mut self, recovery: &mut RecoveryState) -> bool {
        if recovery.continuation_attempts >= MAX_RECOVERY_ATTEMPTS {
            println!(
                "[Error] continuation recovery exhausted after {} attempts",
                MAX_RECOVERY_ATTEMPTS
            );
            return false;
        }

        recovery.continuation_attempts += 1;
        println!(
            "[Recovery] continue ({}/{}): output truncated",
            recovery.continuation_attempts, MAX_RECOVERY_ATTEMPTS
        );
        self.context
            .push(Message::new_text(Role::User, CONTINUATION_MESSAGE));
        true
    }

    async fn maybe_auto_compact(&mut self) -> Result<()> {
        if estimate_context_size(&self.context) <= CONTEXT_THRESHOLD_CHARS {
            return Ok(());
        }

        println!("[Recovery] compact: context estimate exceeded threshold");
        self.compact_history()
            .await
            .context("proactive compact failed")
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

fn is_prompt_too_long_error(error_text: &str) -> bool {
    (error_text.contains("prompt") && error_text.contains("long"))
        || error_text.contains("overlong_prompt")
        || error_text.contains("too many tokens")
        || error_text.contains("context length")
}

fn is_transient_transport_error(error_text: &str) -> bool {
    [
        "timeout",
        "timed out",
        "rate limit",
        "too many requests",
        "unavailable",
        "connection",
        "overloaded",
        "temporarily",
        "econnreset",
        "broken pipe",
    ]
    .iter()
    .any(|needle| error_text.contains(needle))
}

fn backoff_delay(attempt: u32) -> Duration {
    let base = (BACKOFF_BASE_DELAY_SECS * 2f64.powi(attempt as i32)).min(BACKOFF_MAX_DELAY_SECS);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| (duration.subsec_millis() % 1000) as f64 / 1000.0)
        .unwrap_or(0.0);
    Duration::from_secs_f64(base + jitter)
}

fn estimate_context_size(messages: &[Message]) -> usize {
    serde_json::to_string(messages)
        .map(|serialized| serialized.chars().count())
        .unwrap_or_default()
}
