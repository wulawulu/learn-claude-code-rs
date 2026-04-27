use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ToolSpec;
use crate::background::SharedBackgroundManager;
use crate::cron::SharedCronScheduler;
use crate::memory::MemoryManager;
use crate::skill::SkillRegistry;
use crate::task::SharedTaskManager;
use crate::team::SharedTeammateManager;
use crate::worktree::SharedWorktreeManager;
use anyhow::{Context as AnyhowContext, Result};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde_json::Value;

mod background;
mod bash;
mod compact;
mod cron;
mod edit_file;
mod load_skill;
mod math;
mod memory;
mod read_file;
mod subagent;
mod task;
mod team;
mod worktree;
mod write_file;
use background::{BackgroundRunTool, CheckBackgroundTool};
use bash::BashTool;
use compact::CompactTool;
use cron::{CronCreateTool, CronDeleteTool, CronListTool};
use edit_file::EditFileTool;
use load_skill::LoadSkillTool;
use math::AddTool;
use memory::SaveMemoryTool;
use read_file::ReadFileTool;
use subagent::TaskTool;
use task::{TaskCreateTool, TaskGetTool, TaskListTool, TaskUpdateTool};
use team::{
    BroadcastTool, ListTeammatesTool, PlanApprovalTool, ReadInboxTool, SendMessageTool,
    ShutdownRequestTool, ShutdownResponseTool, SpawnTeammateTool,
};
use worktree::{
    WorktreeCreateTool, WorktreeEventsTool, WorktreeListTool, WorktreeRunTool, WorktreeStatusTool,
};
use write_file::WriteFileTool;

#[derive(Clone)]
pub struct ToolContext {
    pub skill_registry: Arc<SkillRegistry>,
    pub memory_manager: Arc<std::sync::Mutex<MemoryManager>>,
    pub work_dir: PathBuf,
    pub task_manager: SharedTaskManager,
    pub background_manager: SharedBackgroundManager,
    pub cron_scheduler: SharedCronScheduler,
    pub teammate_manager: SharedTeammateManager,
    pub worktree_manager: SharedWorktreeManager,
}

pub fn toolset() -> ToolRouter {
    ToolRouter::new()
        .route(AddTool)
        .route(BashTool)
        .route(BackgroundRunTool)
        .route(CheckBackgroundTool)
        .route(CronCreateTool)
        .route(CronDeleteTool)
        .route(CronListTool)
        .route(ReadFileTool)
        .route(WriteFileTool)
        .route(EditFileTool)
        .route(LoadSkillTool)
        .route(SaveMemoryTool)
        .route(CompactTool)
        .route(TaskTool)
        .route(TaskCreateTool)
        .route(TaskGetTool)
        .route(TaskListTool)
        .route(TaskUpdateTool)
        .route(SpawnTeammateTool)
        .route(ListTeammatesTool)
        .route(SendMessageTool)
        .route(BroadcastTool)
        .route(ReadInboxTool)
        .route(PlanApprovalTool)
        .route(ShutdownRequestTool)
        .route(ShutdownResponseTool)
        .route(WorktreeCreateTool)
        .route(WorktreeListTool)
        .route(WorktreeStatusTool)
        .route(WorktreeRunTool)
        .route(WorktreeEventsTool)
}

pub fn subagent_toolset() -> ToolRouter {
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
    use crate::{
        background::SharedBackgroundManager,
        cron::{CronScheduler, SharedCronScheduler},
        memory::MemoryManager,
        store::StoreRoot,
        task::{SharedTaskManager, TaskManager},
        team::{SharedTeammateManager, TeammateManager},
        worktree::{SharedWorktreeManager, WorktreeManager},
    };

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
        let context = test_context("router_dispatches_by_tool_name");

        let output = router
            .call(&context, "echo", serde_json::json!({ "text": "tool" }))
            .await
            .unwrap();

        assert!(output.ends_with(":tool"));
        assert!(output.contains("sfull-tool-test-router_dispatches_by_tool_name"));
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
        let context = test_context("proc_macro_supports_plain_function_tools");

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

    fn test_context(name: &str) -> ToolContext {
        let root_dir = std::env::temp_dir().join(format!("sfull-tool-test-{name}"));
        let _ = std::fs::remove_dir_all(&root_dir);
        std::fs::create_dir_all(&root_dir).unwrap();
        let store_root = StoreRoot::new(root_dir.join(".claude")).unwrap();

        ToolContext {
            skill_registry: Arc::new(SkillRegistry::new(root_dir.join("skills"))),
            memory_manager: Arc::new(std::sync::Mutex::new(MemoryManager::new(
                root_dir.join(".claude/memory"),
            ))),
            work_dir: root_dir.clone(),
            task_manager: SharedTaskManager::new(TaskManager::new(&store_root).unwrap()),
            background_manager: SharedBackgroundManager::new(&store_root).unwrap(),
            cron_scheduler: SharedCronScheduler::new(CronScheduler::new(&store_root).unwrap()),
            teammate_manager: SharedTeammateManager::new(
                TeammateManager::new(&store_root).unwrap(),
            ),
            worktree_manager: SharedWorktreeManager::new(
                WorktreeManager::new(&store_root, root_dir).unwrap(),
            ),
        }
    }
}
