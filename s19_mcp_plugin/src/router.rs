use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::{ToolSpec, mcp_client::McpClient};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolName {
    pub server: String,
    pub tool: String,
}

impl TryFrom<&str> for McpToolName {
    type Error = anyhow::Error;

    fn try_from(tool_name: &str) -> Result<Self> {
        let Some(rest) = tool_name.strip_prefix("mcp__") else {
            bail!("not an MCP tool name: {tool_name}");
        };
        let Some((server, tool)) = rest.rsplit_once("__") else {
            bail!("invalid MCP tool name: {tool_name}");
        };
        if server.is_empty() || tool.is_empty() {
            bail!("invalid MCP tool name: {tool_name}");
        }

        Ok(Self {
            server: server.to_string(),
            tool: tool.to_string(),
        })
    }
}

#[derive(Default)]
pub struct MCPToolRouter {
    clients: HashMap<String, McpClient>,
}

impl MCPToolRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_client(&mut self, client: McpClient) {
        self.clients.insert(client.server_name.clone(), client);
    }

    pub fn is_mcp_tool(tool_name: &str) -> bool {
        tool_name.starts_with("mcp__")
    }

    pub async fn call(&mut self, tool_name: &str, arguments: Value) -> Result<String> {
        let parsed = McpToolName::try_from(tool_name)?;
        let client = self
            .clients
            .get_mut(&parsed.server)
            .with_context(|| format!("unknown MCP server {}", parsed.server))?;

        client.call_tool(&parsed.tool, arguments).await
    }

    pub fn all_tools(&self) -> Vec<ToolSpec> {
        self.clients
            .values()
            .flat_map(McpClient::agent_tools)
            .collect()
    }

    pub fn server_summaries(&self) -> Vec<(String, usize)> {
        let mut summaries = self
            .clients
            .iter()
            .map(|(name, client)| (name.clone(), client.tool_count()))
            .collect::<Vec<_>>();
        summaries.sort_by(|a, b| a.0.cmp(&b.0));
        summaries
    }

    pub async fn disconnect_all(&mut self) {
        for (_, client) in self.clients.drain() {
            client.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::McpToolName;

    #[test]
    fn parses_mcp_tool_name_with_plugin_server_prefix() {
        let parsed = McpToolName::try_from("mcp__demo__postgres__query").unwrap();

        assert_eq!(parsed.server, "demo__postgres");
        assert_eq!(parsed.tool, "query");
    }

    #[test]
    fn rejects_invalid_mcp_tool_name() {
        assert!(McpToolName::try_from("mcp__missing_tool").is_err());
        assert!(McpToolName::try_from("bash").is_err());
    }
}
