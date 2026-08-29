use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ToolSpec;
use anyhow::{Context, Result};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde_json::Value;

mod bash;
mod edit_file;
mod read_file;
mod write_file;

use bash::BashTool;
use edit_file::EditFileTool;
use read_file::ReadFileTool;
use write_file::WriteFileTool;

#[derive(Clone)]
pub struct ToolContext {
    pub work_dir: PathBuf,
}

pub fn toolset() -> ToolRouter {
    ToolRouter::new()
        .route(BashTool)
        .route(ReadFileTool)
        .route(WriteFileTool)
        .route(EditFileTool)
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;
    async fn call(&self, context: ToolContext, input: Value) -> Result<String>;

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: Some(self.description().to_string()),
            input_schema: self.input_schema(),
        }
    }
}

pub struct ToolRouter {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRouter {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn route<T: Tool + 'static>(mut self, tool: T) -> Self {
        self.tools.insert(tool.name().to_string(), Box::new(tool));
        self
    }

    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|tool| tool.tool_spec()).collect()
    }

    pub async fn call(&self, context: &ToolContext, name: &str, input: Value) -> Result<String> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {name}"))?;
        tool.call(context.clone(), input).await
    }
}

impl Default for ToolRouter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn input_schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema generation should not fail")
}

fn safe_path(work_dir: &Path, path: &str) -> Result<PathBuf> {
    resolve_safe_path(work_dir, path, false)
}

fn safe_path_allow_missing(work_dir: &Path, path: &str) -> Result<PathBuf> {
    resolve_safe_path(work_dir, path, true)
}

fn resolve_safe_path(work_dir: &Path, path: &str, allow_missing: bool) -> Result<PathBuf> {
    let work_dir = work_dir.canonicalize()?;
    let candidate = work_dir.join(path);

    let full = if candidate.exists() || !allow_missing {
        candidate.canonicalize()?
    } else {
        let parent = candidate
            .parent()
            .context("path has no parent")?
            .canonicalize()?;
        if !parent.starts_with(&work_dir) {
            anyhow::bail!("path escapes workspace");
        }
        parent.join(candidate.file_name().context("path has no file name")?)
    };

    if !full.starts_with(&work_dir) {
        anyhow::bail!("path escapes workspace");
    }
    Ok(full)
}
