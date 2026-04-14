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
use inquire::Text;

use s02_tool_use::tool::{Tool, bash_tool, edit_file_tool, read_file_tool, write_file_tool};

const MODEL: &str = "deepseek-chat";
const SYSTEM: &str = r#"You are a coding agent.
Use bash to inspect and change the workspace. Act first, then report clearly.
"#;

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
    ]);

    let mut state = LoopState::new(client.clone(), tools);

    loop {
        let query = Text::new("--- How can I help you?")
            .prompt()
            .context("An error happened or user cancelled the input.")?;

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
}

impl LoopState {
    fn new(client: AnthropicClient, tools: HashMap<String, Box<dyn Tool>>) -> Self {
        Self {
            client,
            context: Vec::new(),
            tools,
        }
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

async fn execute_tool_call(
    tools: &HashMap<String, Box<dyn Tool>>,
    content: &[ContentBlock],
) -> Vec<ContentBlock> {
    let mut result = Vec::new();
    for block in content {
        if let ContentBlock::ToolUse { id, name, input } = block {
            let Some(tool) = tools.get(name) else {
                result.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: format!("Unknown tool: {}", name),
                });
                continue;
            };

            match tool.invoke(input).await {
                Ok(output) => {
                    println!("Command:{}\n arg:{}\n output:\n{}\n", name, input, output);
                    result.push(ContentBlock::ToolResult {
                        tool_use_id: id.clone(),
                        content: output,
                    });
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
    result
}

use std::collections::HashSet;

pub fn normalize_messages(messages: &[Message]) -> Vec<Message> {
    let mut messages = messages.to_vec();

    // 1. 收集已有 tool_result
    let mut existing_results = HashSet::new();
    for msg in &messages {
        if let MessageContent::Blocks { content } = &msg.content {
            for block in content {
                if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                    existing_results.insert(tool_use_id.clone());
                }
            }
        }
    }

    // 2. 查找 orphan tool_use
    let mut extra_messages = Vec::new();

    for msg in &messages {
        if matches!(msg.role, Role::User) {
            continue;
        }

        if let MessageContent::Blocks { content } = &msg.content {
            for block in content {
                if let ContentBlock::ToolUse { id, .. } = block
                    && !existing_results.contains(id)
                {
                    extra_messages.push(Message::new_blocks(
                        Role::User,
                        vec![ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: "(cancelled)".to_string(),
                        }],
                    ));
                }
            }
        }
    }
    messages.extend(extra_messages);

    // 3. 合并连续相同 role
    let mut merged: Vec<Message> = Vec::new();
    for msg in messages {
        if let Some(last) = merged.last_mut()
            && matches!(
                (last.role, msg.role),
                (Role::User, Role::User) | (Role::Assistant, Role::Assistant)
            )
        {
            // 合并 content
            match (&mut last.content, msg.content) {
                (
                    MessageContent::Blocks { content: prev },
                    MessageContent::Blocks { content: curr },
                ) => {
                    prev.extend(curr);
                }
                (
                    MessageContent::Text { content: prev },
                    MessageContent::Text { content: curr },
                ) => {
                    prev.push('\n');
                    prev.push_str(&curr);
                }
                (
                    MessageContent::Text { content: prev },
                    MessageContent::Blocks { content: curr },
                ) => {
                    let mut new_blocks = vec![ContentBlock::Text { text: prev.clone() }];
                    new_blocks.extend(curr);
                    last.content = MessageContent::Blocks {
                        content: new_blocks,
                    };
                }
                (
                    MessageContent::Blocks { content: prev },
                    MessageContent::Text { content: curr },
                ) => {
                    prev.push(ContentBlock::Text { text: curr });
                }
            }
            continue;
        }
        merged.push(msg);
    }

    merged
}

async fn agent_loop(state: &mut LoopState) -> Result<()> {
    loop {
        let request = CreateMessageParams::new(RequiredMessageParams {
            model: MODEL.to_string(),
            messages: normalize_messages(&state.context),
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

        let tool_result = execute_tool_call(&state.tools, &response.content).await;

        state
            .context
            .push(Message::new_blocks(Role::User, tool_result));
    }
}
