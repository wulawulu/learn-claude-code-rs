use std::process::Stdio;

use anyhow::{Context, Result};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, RawContent, ResourceContents, Tool as McpTool},
    service::RunningService,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::{Map, Value};
use tokio::process::Command;

use crate::{ToolSpec, plugin::McpServerConfig};

pub struct McpClient {
    pub server_name: String,
    service: RunningService<RoleClient, ()>,
    tools: Vec<McpTool>,
}

impl McpClient {
    pub async fn try_new(server_name: impl Into<String>, config: McpServerConfig) -> Result<Self> {
        let server_name = server_name.into();
        let service = Self::connect(&server_name, config).await?;
        match Self::fetch_tools(&server_name, &service).await {
            Ok(tools) => Ok(Self {
                server_name,
                service,
                tools,
            }),
            Err(err) => {
                let _ = service.cancel().await;
                Err(err)
            }
        }
    }

    pub fn list_tools(&self) -> &[McpTool] {
        &self.tools
    }

    async fn connect(
        server_name: &str,
        config: McpServerConfig,
    ) -> Result<RunningService<RoleClient, ()>> {
        let command = config.command;
        let args = config.args;
        let env = config.env;
        let transport = TokioChildProcess::builder(Command::new(&command).configure(move |cmd| {
            cmd.args(&args).envs(&env).stderr(Stdio::inherit());
        }))
        .spawn()
        .with_context(|| format!("failed to spawn MCP server {server_name}"))?
        .0;

        ().serve(transport)
            .await
            .with_context(|| format!("failed to initialize MCP client for server {server_name}"))
    }

    async fn fetch_tools(
        server_name: &str,
        service: &RunningService<RoleClient, ()>,
    ) -> Result<Vec<McpTool>> {
        let tools = service
            .peer()
            .list_all_tools()
            .await
            .with_context(|| format!("failed to list tools from {server_name}"))?;

        Ok(tools)
    }

    pub async fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<String> {
        let arguments = match arguments {
            Value::Object(map) => Some(map),
            Value::Null => None,
            other => {
                let mut map = Map::new();
                map.insert("value".to_string(), other);
                Some(map)
            }
        };

        let result = self
            .service
            .peer()
            .call_tool(CallToolRequestParams {
                meta: None,
                name: tool_name.to_string().into(),
                arguments,
                task: None,
            })
            .await
            .with_context(|| format!("failed to call MCP tool {tool_name}"))?;

        Ok(join_mcp_content(&result.content))
    }

    pub fn agent_tools(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .map(|tool| ToolSpec {
                name: format!("mcp__{}__{}", self.server_name, tool.name),
                description: tool.description.as_ref().map(ToString::to_string),
                input_schema: Value::Object((*tool.input_schema).clone()),
            })
            .collect()
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    pub async fn shutdown(self) {
        let _ = self.service.cancel().await;
    }
}

fn join_mcp_content(content: &[rmcp::model::Content]) -> String {
    let parts = content
        .iter()
        .map(|content| match &content.raw {
            RawContent::Text(text) => text.text.clone(),
            RawContent::Resource(resource) => match &resource.resource {
                ResourceContents::TextResourceContents { text, .. } => text.clone(),
                _ => String::new(),
            },
            other => serde_json::to_string(other).unwrap_or_else(|_| "<non-text content>".into()),
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    parts.join("\n")
}
