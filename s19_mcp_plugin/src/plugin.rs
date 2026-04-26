use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

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
            let manifest_path = manifest_path(dir);
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

    pub fn plugins(&self) -> &HashMap<String, PluginManifest> {
        &self.plugins
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

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join(".claude-plugin").join("plugin.json")
}

#[cfg(test)]
mod tests {
    use super::{McpServerConfig, PluginManifest};

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
}
