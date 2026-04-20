use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::{ToolSpec, team::SharedTeammateManager, tool::Tool};

pub struct ListTeammatesTool {
    manager: SharedTeammateManager,
}

pub fn list_teammates_tool(manager: SharedTeammateManager) -> Box<dyn Tool> {
    Box::new(ListTeammatesTool { manager }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for ListTeammatesTool {
    async fn invoke(&mut self, _input: &Value) -> Result<String> {
        self.manager.list_all()
    }

    fn name(&self) -> Cow<'_, str> {
        "list_teammates".into()
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_teammates".to_string(),
            description: Some("List all teammates with name, role, status.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }
}
