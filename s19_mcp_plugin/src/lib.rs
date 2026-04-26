pub mod mcp_client;
pub mod permission;
pub mod plugin;
pub mod router;
pub mod tool;
pub use anthropic_ai_sdk::types::message::Tool as ToolSpec;
use inquire::Select;
use serde::Serialize;
use serde_json::Value;

use std::collections::HashMap;

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient as _, MessageContent,
        MessageError, RequiredMessageParams, Role, StopReason,
    },
};
use anyhow::{Context, Result};

use crate::{
    mcp_client::McpClient,
    permission::{
        CapabilityIntent, CapabilitySource,
        PermissionBehavior::{Allow, Ask, Deny},
        PermissionDecision, PermissionManager, PermissionMode, normalize_capability,
    },
    plugin::PluginLoader,
    router::MCPToolRouter,
    tool::Tool,
};

pub const MODEL: &str = "deepseek-chat";

pub fn get_llm_client() -> Result<AnthropicClient> {
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
    pub mcp_router: MCPToolRouter,
    pub permission_manager: PermissionManager,
}

impl LoopState {
    pub fn new(
        client: AnthropicClient,
        tools: HashMap<String, Box<dyn Tool>>,
        mcp_router: MCPToolRouter,
        permission_manager: PermissionManager,
    ) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
            mcp_router,
            permission_manager,
        }
    }

    pub async fn agent_loop(&mut self) -> Result<()> {
        let system = format!(
            r#"You are a coding agent at {}. Use tools to solve tasks.
The user controls permissions. Some tool calls may be denied."#,
            std::env::current_dir()?.display(),
        );
        loop {
            let request = CreateMessageParams::new(RequiredMessageParams {
                model: MODEL.to_string(),
                messages: self.context.clone(),
                max_tokens: 8000,
            })
            .with_system(&system)
            .with_tools(self.all_tool_specs());

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

    pub async fn execute_tool_call(&mut self, content: &[ContentBlock]) -> Result<()> {
        let mut result = Vec::new();
        for block in content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                // Check permission
                let decision = self.permission_manager.check(name, input);
                let output;
                match decision {
                    PermissionDecision {
                        behavior: Deny,
                        reason,
                    } => {
                        output = ToolResultPayload::permission_denied(
                            normalize_capability(name, input),
                            format!("Permission denied: {reason}"),
                        )
                        .to_json_string();
                        println!("  [DENIED] {}: {}", name, reason);
                    }
                    PermissionDecision {
                        behavior: Allow,
                        reason: _,
                    } => {
                        output = self.execute(name, input).await;
                    }
                    PermissionDecision {
                        behavior: Ask,
                        reason: _reason,
                    } => {
                        if self.permission_manager.ask_user(name, input)? {
                            output = self.execute(name, input).await;
                        } else {
                            output = ToolResultPayload::permission_denied(
                                normalize_capability(name, input),
                                format!("Permission denied by user for {name}"),
                            )
                            .to_json_string();
                            println!("  [USER DENIED] {name}");
                        }
                    }
                }
                result.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: output,
                });
            }
        }
        self.context.push(Message::new_blocks(Role::User, result));
        Ok(())
    }

    pub fn handle_mode_command(&mut self, query: &str) -> Result<()> {
        let parts: Vec<&str> = query.split_whitespace().collect::<Vec<_>>();

        let mode = if parts.len() == 2 {
            parts[1].parse::<PermissionMode>().with_context(|| {
                format!(
                    "unknown mode: {}. Usage: /mode <default|plan|auto>",
                    parts[1]
                )
            })?
        } else {
            Select::new(
                "Switch permission mode:",
                vec![
                    PermissionMode::Default,
                    PermissionMode::Plan,
                    PermissionMode::Auto,
                ],
            )
            .prompt()
            .context("An error happened or user cancelled the input.")?
        };

        self.permission_manager.set_mode(mode);
        println!("[Switched to {}]", self.permission_manager.mode());

        Ok(())
    }

    async fn execute(&mut self, name: &str, input: &Value) -> String {
        let intent = normalize_capability(name, input);
        if MCPToolRouter::is_mcp_tool(name) {
            return match self.mcp_router.call(name, input.clone()).await {
                Ok(output) => {
                    println!(
                        "MCP tool:{}\n arg:{}\n output:\n{}\n",
                        name,
                        input,
                        output.chars().take(200).collect::<String>()
                    );
                    ToolResultPayload::ok(intent, output).to_json_string()
                }
                Err(e) => {
                    println!("Error invoking MCP tool {}: {}", name, e);
                    ToolResultPayload::error(intent, e.to_string()).to_json_string()
                }
            };
        }

        let Some(tool) = self.tools.get_mut(name) else {
            return ToolResultPayload::error(intent, format!("Unknown tool: {name}"))
                .to_json_string();
        };

        match tool.invoke(input).await {
            Ok(output) => {
                println!(
                    "Command:{}\n arg:{}\n output:\n{}\n",
                    name,
                    input,
                    output.chars().take(200).collect::<String>()
                );
                ToolResultPayload::ok(intent, output).to_json_string()
            }
            Err(e) => {
                println!("Error invoking tool {}: {}", name, e);
                ToolResultPayload::error(intent, e.to_string()).to_json_string()
            }
        }
    }

    pub fn all_tool_specs(&self) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|tool| tool.tool_spec())
            .chain(self.mcp_router.all_tools())
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Ok,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolResultPayload {
    pub source: String,
    pub server: Option<String>,
    pub tool: String,
    pub risk: String,
    pub status: ToolResultStatus,
    pub preview: String,
}

impl ToolResultPayload {
    const PREVIEW_LIMIT: usize = 500;

    pub fn ok(intent: CapabilityIntent, output: impl AsRef<str>) -> Self {
        Self::from_intent(intent, ToolResultStatus::Ok, output)
    }

    pub fn error(intent: CapabilityIntent, output: impl AsRef<str>) -> Self {
        Self::from_intent(intent, ToolResultStatus::Error, output)
    }

    pub fn permission_denied(intent: CapabilityIntent, reason: impl AsRef<str>) -> Self {
        Self::error(intent, reason)
    }

    pub fn to_json_string(&self) -> String {
        serde_json::to_string(self).expect("tool result payload should serialize")
    }

    fn from_intent(
        intent: CapabilityIntent,
        status: ToolResultStatus,
        output: impl AsRef<str>,
    ) -> Self {
        Self {
            source: intent.source.to_string(),
            server: match intent.source {
                CapabilitySource::Native => None,
                CapabilitySource::Mcp => intent.server,
            },
            tool: intent.tool,
            risk: intent.risk.to_string(),
            status,
            preview: preview(output.as_ref(), Self::PREVIEW_LIMIT),
        }
    }
}

fn preview(output: &str, limit: usize) -> String {
    if output.chars().count() <= limit {
        output.to_string()
    } else {
        output.chars().take(limit).collect()
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

pub async fn load_mcp_router() -> Result<MCPToolRouter> {
    let cwd = std::env::current_dir()?;
    let mut loader = PluginLoader::new(vec![cwd]);
    let plugins = loader.scan()?;
    if plugins.is_empty() {
        println!("[Plugins: none]");
    } else {
        println!("[Plugins: {}]", plugins.join(", "));
    }

    let mut router = MCPToolRouter::new();
    for (server_name, config) in loader.mcp_servers() {
        match McpClient::try_new(server_name.clone(), config).await {
            Ok(client) => {
                println!(
                    "[MCP connected: {server_name} ({} tools)]",
                    client.list_tools().len()
                );
                router.register_client(client);
            }
            Err(err) => {
                println!("[MCP connect failed: {server_name}: {err:#}]");
            }
        }
    }

    Ok(router)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{ToolResultPayload, ToolResultStatus, permission::normalize_capability};

    #[test]
    fn tool_result_payload_serializes_stable_shape() {
        let intent = normalize_capability("mcp__demo__db__query", &json!({"sql": "select 1"}));
        let payload = ToolResultPayload::ok(intent, "rows");
        let value: serde_json::Value = serde_json::from_str(&payload.to_json_string()).unwrap();

        assert_eq!(payload.status, ToolResultStatus::Ok);
        assert_eq!(value["source"], "mcp");
        assert_eq!(value["server"], "demo__db");
        assert_eq!(value["tool"], "query");
        assert_eq!(value["risk"], "read");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["preview"], "rows");
    }

    #[test]
    fn tool_result_payload_truncates_preview() {
        let intent = normalize_capability("write_file", &json!({"path": "a.txt"}));
        let payload = ToolResultPayload::error(intent, "x".repeat(600));

        assert_eq!(payload.status, ToolResultStatus::Error);
        assert_eq!(payload.preview.chars().count(), 500);
    }
}
