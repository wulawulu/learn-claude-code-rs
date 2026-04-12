use std::{borrow::Cow, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, skill::SkillRegistry, tool::Tool};

pub struct LoadSkillTool {
    registry: Arc<SkillRegistry>,
}

pub fn load_skill_tool(registry: Arc<SkillRegistry>) -> Box<dyn Tool> {
    Box::new(LoadSkillTool { registry }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for LoadSkillTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .context("Invalid name")?;

        Ok(self.registry.load_full_text(name))
    }

    fn name(&self) -> Cow<'_, str> {
        "load_skill".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "load_skill".to_string(),
            description: Some(
                "Load the full body of a named skill into the current context.".to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
            }),
        }
    }
}
