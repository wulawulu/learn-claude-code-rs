use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ToolSpec;
use crate::skill::SkillRegistry;
use anyhow::{Context as AnyhowContext, Result};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde_json::Value;

mod bash;
mod edit_file;
mod load_skill;
mod math;
mod read_file;
mod write_file;
use bash::BashTool;
use edit_file::EditFileTool;
use load_skill::LoadSkillTool;
use math::AddTool;
use read_file::ReadFileTool;
use write_file::WriteFileTool;

#[derive(Clone)]
pub struct ToolContext {
    pub skill_registry: Arc<SkillRegistry>,
    pub work_dir: PathBuf,
}

pub fn toolset() -> ToolRouter {
    ToolRouter::new()
        .route(AddTool)
        .route(BashTool)
        .route(ReadFileTool)
        .route(WriteFileTool)
        .route(EditFileTool)
        .route(LoadSkillTool)
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

    pub fn route<T>(mut self, tool: T) -> Self
    where
        T: Tool + 'static,
    {
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

pub fn input_schema<T>() -> Value
where
    T: JsonSchema,
{
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
            .context("Path has no parent")?
            .canonicalize()?;

        if !parent.starts_with(&work_dir) {
            return Err(anyhow::anyhow!("Path escapes workspace"));
        }

        let file_name = candidate.file_name().context("Path has no file name")?;

        parent.join(file_name)
    };

    if !full.starts_with(&work_dir) {
        return Err(anyhow::anyhow!("Path escapes workspace"));
    }

    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize, JsonSchema)]
    struct EchoInput {
        #[schemars(description = "Text to echo.")]
        text: String,
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &'static str {
            "echo"
        }

        fn description(&self) -> &'static str {
            "Echo text with a prefix."
        }

        fn input_schema(&self) -> Value {
            input_schema::<EchoInput>()
        }

        async fn call(&self, context: ToolContext, input: Value) -> Result<String> {
            let input: EchoInput = serde_json::from_value(input)?;
            Ok(format!("{}:{}", context.work_dir.display(), input.text))
        }
    }

    #[tokio::test]
    async fn router_dispatches_by_tool_name() {
        let router = ToolRouter::new().route(EchoTool);
        let context = ToolContext {
            skill_registry: Arc::new(SkillRegistry::new(PathBuf::from("skills"))),
            work_dir: PathBuf::from("workspace"),
        };

        let output = router
            .call(&context, "echo", serde_json::json!({ "text": "tool" }))
            .await
            .unwrap();

        assert_eq!(output, "workspace:tool");
    }

    #[test]
    fn schema_is_generated_from_input_type() {
        let spec = EchoTool.tool_spec();
        let schema = spec.input_schema;

        assert_eq!(schema["title"], "EchoInput");
        assert_eq!(schema["properties"]["text"]["type"], "string");
        assert_eq!(schema["properties"]["text"]["description"], "Text to echo.");
        assert_eq!(schema["required"][0], "text");
    }

    #[tokio::test]
    async fn proc_macro_supports_plain_function_tools() {
        let router = ToolRouter::new().route(AddTool);
        let context = ToolContext {
            skill_registry: Arc::new(SkillRegistry::new(PathBuf::from("skills"))),
            work_dir: PathBuf::from("."),
        };

        let output = router
            .call(&context, "add", serde_json::json!({ "a": 2, "b": 3 }))
            .await
            .unwrap();

        assert_eq!(output, "5");

        let schema = AddTool.tool_spec().input_schema;
        assert_eq!(schema["properties"]["a"]["type"], "integer");
        assert_eq!(
            schema["properties"]["a"]["description"],
            "Left integer operand."
        );
        assert_eq!(schema["properties"]["b"]["type"], "integer");
    }
}
