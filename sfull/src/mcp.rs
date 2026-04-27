use std::{collections::HashMap, fs, path::PathBuf, process::Stdio};

use anyhow::{Context, Result, bail};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, RawContent, ResourceContents, Tool as McpTool},
    service::RunningService,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::process::Command;

use crate::ToolSpec;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Default)]
pub struct PluginLoader {
    search_dirs: Vec<PathBuf>,
    plugins: HashMap<String, PluginManifest>,
}

impl PluginLoader {
    pub fn new(search_dirs: Vec<PathBuf>) -> Self {
        Self {
            search_dirs,
            plugins: HashMap::new(),
        }
    }

    pub fn scan(&mut self) -> Result<Vec<String>> {
        self.plugins.clear();
        let mut loaded = Vec::new();

        for dir in &self.search_dirs {
            let manifest_path = dir.join(".claude-plugin").join("plugin.json");
            if !manifest_path.exists() {
                continue;
            }

            let raw = fs::read_to_string(&manifest_path)
                .with_context(|| format!("failed to read {}", manifest_path.display()))?;
            let manifest: PluginManifest = serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse {}", manifest_path.display()))?;

            loaded.push(manifest.name.clone());
            self.plugins.insert(manifest.name.clone(), manifest);
        }

        Ok(loaded)
    }

    pub fn mcp_servers(&self) -> HashMap<String, McpServerConfig> {
        let mut servers = HashMap::new();
        for (plugin_name, manifest) in &self.plugins {
            for (server_name, config) in &manifest.mcp_servers {
                servers.insert(format!("{plugin_name}__{server_name}"), config.clone());
            }
        }
        servers
    }
}

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
        service
            .peer()
            .list_all_tools()
            .await
            .with_context(|| format!("failed to list tools from {server_name}"))
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

#[cfg(test)]
mod tests {
    use super::{McpServerConfig, McpToolName, PluginManifest};

    #[test]
    fn parses_plugin_manifest() {
        let raw = r#"{
          "name": "demo",
          "version": "1.0.0",
          "mcpServers": {
            "echo": {
              "command": "node",
              "args": ["server.js"],
              "env": {"A": "B"}
            }
          }
        }"#;

        let manifest: PluginManifest = serde_json::from_str(raw).unwrap();
        let expected = McpServerConfig {
            command: "node".to_string(),
            args: vec!["server.js".to_string()],
            env: [("A".to_string(), "B".to_string())].into(),
        };

        assert_eq!(manifest.name, "demo");
        assert_eq!(manifest.version.as_deref(), Some("1.0.0"));
        assert_eq!(manifest.mcp_servers["echo"].command, expected.command);
        assert_eq!(manifest.mcp_servers["echo"].args, expected.args);
        assert_eq!(manifest.mcp_servers["echo"].env, expected.env);
    }

    #[test]
    fn parses_mcp_tool_name_with_plugin_server_prefix() {
        let parsed = McpToolName::try_from("mcp__demo__postgres__query").unwrap();

        assert_eq!(parsed.server, "demo__postgres");
        assert_eq!(parsed.tool, "query");
    }
}
