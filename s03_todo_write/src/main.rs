use std::collections::HashMap;

use anthropic_ai_sdk::{
    client::{AnthropicClient, AnthropicClientBuilder},
    types::message::{
        ContentBlock, CreateMessageParams, Message, MessageClient, MessageContent, MessageError,
        RequiredMessageParams,
        Role::{self, User},
        StopReason,
    },
};
use anyhow::{Context, Result};

use s03_todo_write::tool::{Tool, bash_tool, edit_file_tool, read_file_tool, write_file_tool};

const MODEL: &str = "deepseek-chat";
const SYSTEM: &str = r#"You are a coding agent.
Use the todo tool for multi-step work.
Keep exactly one step in_progress when a task has multiple steps.
Refresh the plan as work advances. Prefer tools over prose.
"#;

const PLAN_REMINDER_INTERVAL: usize = 3;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let anthropic_api_key =
        std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY is not set")?;
    let anthropic_base_url =
        std::env::var("ANTHROPIC_BASE_URL").context("ANTHROPIC_BASE_URL is not set")?;
    let client = AnthropicClientBuilder::new(anthropic_api_key, "")
        .with_api_base_url(anthropic_base_url)
        .build::<MessageError>()
        .context("can't create client")?;

    let tools = HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("write_file".to_string(), write_file_tool()),
        ("edit_file".to_string(), edit_file_tool()),
        ("todo".to_string(), s03_todo_write::tool::todo_tool()),
    ]);

    let mut state = LoopState::new(client.clone(), tools);

    loop {
        println!("--- How can I help you?");
        //get user input
        let mut query = String::new();
        std::io::stdin()
            .read_line(&mut query)
            .context("Failed to read user input")?;

        //break out of the loop if the user enters exit()
        if query.trim() == "exit()" {
            break;
        }
        state.context.push(Message::new_text(User, query));

        agent_loop(&mut state).await?;

        let Some(final_content) = state.context.last() else {
            continue;
        };
        println!(
            "--- Final response:\n{}",
            extract_text(&final_content.content)
        );
    }

    Ok(())
}

struct LoopState {
    client: AnthropicClient,
    pub context: Vec<Message>,
    tools: HashMap<String, Box<dyn Tool>>,
    todo_rounds_since_update: usize,
}

impl LoopState {
    fn new(client: AnthropicClient, tools: HashMap<String, Box<dyn Tool>>) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
            todo_rounds_since_update: 0,
        }
    }

    async fn execute_tool_call(&mut self, content: &[ContentBlock]) -> Vec<ContentBlock> {
        let mut result = Vec::new();
        let mut used_todo = false;
        for block in content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                let Some(tool) = self.tools.get_mut(name) else {
                    result.push(ContentBlock::ToolResult {
                        tool_use_id: id.clone(),
                        content: format!("Unknown tool: {}", name),
                    });
                    continue;
                };

                match tool.invoke(input).await {
                    Ok(output) => {
                        println!(
                            "Command:{}\n arg:{}\n output:\n{}\n",
                            name,
                            input,
                            output.chars().take(200).collect::<String>()
                        );
                        result.push(ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: output,
                        });
                        if name == "todo" {
                            used_todo = true;
                        }
                    }
                    Err(e) => {
                        println!("Error invoking tool {}: {}", name, e);
                        result.push(ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: format!("Error invoking tool {}: {}", name, e),
                        });
                    }
                }
            }
        }
        if used_todo {
            self.todo_rounds_since_update = 0;
        } else {
            self.note_round_without_update();
            if let Some(reminder) = self.reminder() {
                result.insert(0, ContentBlock::Text { text: reminder });
            }
        }
        result
    }

    pub fn reminder(&mut self) -> Option<String> {
        if self.todo_rounds_since_update >= PLAN_REMINDER_INTERVAL {
            Some("<reminder>Refresh your current plan before continuing.</reminder>".into())
        } else {
            None
        }
    }

    pub fn note_round_without_update(&mut self) {
        self.todo_rounds_since_update += 1;
    }
}

fn extract_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { content } => content.clone(),
        MessageContent::Blocks { content } => content
            .iter()
            .filter_map(|block| {
                if let ContentBlock::Text { text } = block {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

async fn agent_loop(state: &mut LoopState) -> Result<()> {
    loop {
        let request = CreateMessageParams::new(RequiredMessageParams {
            model: MODEL.to_string(),
            messages: state.context.clone(),
            max_tokens: 8000,
        })
        .with_system(SYSTEM)
        .with_tools(state.tools.values().map(|tool| tool.tool_spec()).collect());

        let response = state.client.create_message(Some(&request)).await?;

        state.context.push(Message::new_blocks(
            Role::Assistant,
            response.content.clone(),
        ));

        if let Some(stop_reason) = response.stop_reason
            && !matches!(stop_reason, StopReason::ToolUse)
        {
            return Ok(());
        }

        let tool_result = state.execute_tool_call(&response.content).await;

        state
            .context
            .push(Message::new_blocks(Role::User, tool_result));
    }
}
