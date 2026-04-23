use std::path::PathBuf;
use std::{borrow::Cow, path::Path};

use crate::{
    LoopState, ToolSpec, canonical_work_dir, extract_text, get_llm_client,
    tool::{Tool, subagent_toolset},
    worktree::SharedWorktreeManager,
};
use anthropic_ai_sdk::types::message::{Message, Role};
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct SubAgentTool {
    worktrees: SharedWorktreeManager,
    work_dir: PathBuf,
}

pub fn sub_agent_tool(worktrees: SharedWorktreeManager, work_dir: PathBuf) -> Box<dyn Tool> {
    Box::new(SubAgentTool {
        worktrees,
        work_dir,
    }) as Box<dyn Tool>
}

#[derive(Debug, Clone)]
struct SubagentTarget {
    task_id: Option<u64>,
    worktree: Option<String>,
    work_dir: PathBuf,
}

impl SubAgentTool {
    async fn run_subagent(
        &self,
        description: Option<&str>,
        prompt: &str,
        target: SubagentTarget,
    ) -> Result<String> {
        let worktree_name = target.worktree.clone();
        let task_id = target.task_id;

        if let Some(worktree_name) = worktree_name.as_deref() {
            let _ = self.worktrees.enter(worktree_name);
            let _ = self.worktrees.record_subagent_event(
                "worktree.subagent.start",
                task_id,
                worktree_name,
                description,
            );
        }

        println!(
            "> sub_agent {} @ {}",
            description.unwrap_or_default(),
            target.work_dir.display()
        );

        let client = get_llm_client()?;
        let tools = subagent_toolset(target.work_dir.clone());
        let mut state = LoopState::new(client, tools, build_system_prompt(&target.work_dir), 30);

        if let Some(worktree_name) = worktree_name.as_deref() {
            let metadata = json!({
                "worktree": worktree_name,
                "task_id": task_id,
                "workspace": target.work_dir.display().to_string(),
            });
            state.context.push(Message::new_text(
                Role::User,
                format!("<lane>{}</lane>", serde_json::to_string_pretty(&metadata)?),
            ));
        }

        state.context.push(Message::new_text(Role::User, prompt));
        let result = state.agent_loop().await;

        let summary = state
            .context
            .iter()
            .rev()
            .find(|message| matches!(message.role, Role::Assistant))
            .map(|message| extract_text(&message.content))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "(no summary)".to_string());

        if let Some(worktree_name) = worktree_name.as_deref() {
            let event = if result.is_ok() {
                "worktree.subagent.finish"
            } else {
                "worktree.subagent.failed"
            };
            let _ =
                self.worktrees
                    .record_subagent_event(event, task_id, worktree_name, description);
        }

        result?;
        Ok(summary)
    }

    fn resolve_target(&self, input: &Value) -> Result<SubagentTarget> {
        let requested_worktree = input.get("worktree").and_then(|value| value.as_str());
        let task_id = input.get("task_id").and_then(|value| value.as_u64());

        match (requested_worktree, task_id) {
            (Some(name), Some(task_id)) => {
                let (bound_name, path) = self.worktrees.path_for_task(task_id)?;
                if bound_name != name {
                    anyhow::bail!(
                        "Task {} is bound to worktree '{}' instead of '{}'",
                        task_id,
                        bound_name,
                        name
                    );
                }
                Ok(SubagentTarget {
                    task_id: Some(task_id),
                    worktree: Some(name.to_string()),
                    work_dir: canonical_work_dir(path)?,
                })
            }
            (Some(name), None) => {
                let record = self.worktrees.get_record(name)?;
                let path = self.worktrees.path_for(name)?;
                Ok(SubagentTarget {
                    task_id: record.task_id,
                    worktree: Some(name.to_string()),
                    work_dir: canonical_work_dir(path)?,
                })
            }
            (None, Some(task_id)) => {
                let (name, path) = self.worktrees.path_for_task(task_id)?;
                Ok(SubagentTarget {
                    task_id: Some(task_id),
                    worktree: Some(name),
                    work_dir: canonical_work_dir(path)?,
                })
            }
            (None, None) => Ok(SubagentTarget {
                task_id: None,
                worktree: None,
                work_dir: self.work_dir.clone(),
            }),
        }
    }
}

#[async_trait]
impl Tool for SubAgentTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .context("Invalid prompt")?;
        let description = input.get("description").and_then(|v| v.as_str());

        let target = self.resolve_target(input)?;
        self.run_subagent(description, prompt, target).await
    }

    fn name(&self) -> Cow<'_, str> {
        "sub_agent".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "sub_agent".to_string(),
            description: Some(
                "Spawn a fresh subagent. If task_id or worktree is provided, the child runs inside that isolated worktree lane."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string"},
                    "description": {"type": "string"},
                    "task_id": {"type": "integer"},
                    "worktree": {"type": "string"}
                },
                "required": ["prompt"]
            }),
        }
    }
}

fn build_system_prompt(work_dir: &Path) -> String {
    format!(
        "You are a coding subagent. Make the requested changes directly, then summarize the outcome. Your scoped workspace is {}. Treat this directory as your entire working lane. Never use parent traversal or absolute paths to escape it. Return a concise final summary.",
        work_dir.display()
    )
}
