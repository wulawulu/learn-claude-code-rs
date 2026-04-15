use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    ToolSpec,
    memory::{MemoryManager, MemoryType},
    tool::Tool,
};

pub struct SaveMemoryTool {
    memory_manager: Arc<Mutex<MemoryManager>>,
}

#[derive(Deserialize)]
struct SaveMemoryInput {
    name: String,
    description: String,
    #[serde(rename = "type")]
    memory_type: String,
    content: String,
}

pub fn save_memory_tool(memory_manager: Arc<Mutex<MemoryManager>>) -> Box<dyn Tool> {
    Box::new(SaveMemoryTool { memory_manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for SaveMemoryTool {
    async fn invoke(&mut self, input: &Value) -> Result<String> {
        let input: SaveMemoryInput =
            serde_json::from_value(input.clone()).context("Invalid save_memory input")?;
        let memory_type = input.memory_type.parse::<MemoryType>()?;

        let mut memory_manager = self
            .memory_manager
            .lock()
            .map_err(|_| anyhow::anyhow!("memory manager lock poisoned"))?;

        memory_manager.save_memory(&input.name, &input.description, memory_type, &input.content)
    }

    fn name(&self) -> Cow<'_, str> {
        "save_memory".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "save_memory".to_string(),
            description: Some(
                "Save a persistent memory that survives across sessions.".to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Short identifier (e.g. prefer_tabs, db_schema)"
                    },
                    "description": {
                        "type": "string",
                        "description": "One-line summary of what this memory captures"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["user", "feedback", "project", "reference"],
                        "description": "user=preferences, feedback=corrections, project=non-obvious project conventions or decision reasons, reference=external resource pointers"
                    },
                    "content": {
                        "type": "string",
                        "description": "Full memory content (multi-line OK)"
                    }
                },
                "required": ["name", "description", "type", "content"]
            }),
        }
    }
}
