use std::{borrow::Cow, collections::HashMap};

use crate::{
    LoopState, MODEL, ToolSpec, extract_text, get_llm_client,
    tool::{Tool, bash_tool, edit_file_tool, read_file_tool, write_file_tool},
};
use anthropic_ai_sdk::types::message::{
    CreateMessageParams, Message, MessageClient, RequiredMessageParams, Role, StopReason,
};
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

pub struct SubAgentTool;

pub fn sub_agent_tool() -> Box<dyn Tool> {
    Box::new(SubAgentTool {}) as Box<dyn Tool>
}

async fn sub_agent_loop(prompt: &str, description: Option<&str>) -> Result<String> {
    println!("> task - ({}): {}", description.unwrap_or_default(), prompt);
    let client = get_llm_client()?;
    let tools = HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("write_file".to_string(), write_file_tool()),
        ("edit_file".to_string(), edit_file_tool()),
    ]);

    let mut state = LoopState::new(client.clone(), tools);
    state.context.push(Message::new_text(Role::User, prompt));

    let mut last_summary = None;

    let sub_system = format!(
        "You are a coding subagent at {}. Complete the given task, then summarize your findings.",
        std::env::current_dir()?.display()
    );
    for _ in 0..30 {
        let request = CreateMessageParams::new(RequiredMessageParams {
            model: MODEL.to_string(),
            messages: state.context.clone(),
            max_tokens: 8000,
        })
        .with_system(&sub_system)
        .with_tools(state.tools.values().map(|tool| tool.tool_spec()).collect());

        let response = state.client.create_message(Some(&request)).await?;
        let response_text =
            extract_text(&Message::new_blocks(Role::Assistant, response.content.clone()).content);

        if !response_text.is_empty() {
            last_summary = Some(response_text);
        }

        state.context.push(Message::new_blocks(
            Role::Assistant,
            response.content.clone(),
        ));

        if let Some(stop_reason) = response.stop_reason
            && !matches!(stop_reason, StopReason::ToolUse)
        {
            break;
        }

        let tool_result = state.execute_tool_call(&response.content).await;

        state
            .context
            .push(Message::new_blocks(Role::User, tool_result));
    }

    Ok(last_summary.unwrap_or_else(|| "(no summary)".to_string()))
}

#[async_trait]
impl Tool for SubAgentTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .context("Invalid prompt")?;
        let description = input.get("description").and_then(|v| v.as_str());
        sub_agent_loop(prompt, description).await
    }

    fn name(&self) -> Cow<'_, str> {
        "task".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "task".to_string(),
            description: Some("Spawn a subagent with fresh context. It shares the filesystem but not conversation history.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string"},
                    "description": {"type": "string", "description": "Short description of the task"}
                },
                "required": ["prompt"]
            }),
        }
    }
}
