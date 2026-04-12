use std::{collections::HashMap, sync::Arc};

use anthropic_ai_sdk::types::message::{
    CreateMessageParams, Message, MessageClient, RequiredMessageParams,
    Role::{self, User},
    StopReason,
};
use anyhow::{Context, Result};

use s05_skill_loading::{
    LoopState, MODEL, extract_text, get_llm_client,
    skill::get_skill_registry,
    tool::{bash_tool, edit_file_tool, load_skill_tool, read_file_tool, write_file_tool},
};

const SKILLS_DIR: &str = "skills";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;

    let skills_dir = std::env::current_dir()?.join(SKILLS_DIR);
    let skill_registry = Arc::new(get_skill_registry(skills_dir)?);

    let tools = HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("write_file".to_string(), write_file_tool()),
        ("edit_file".to_string(), edit_file_tool()),
        (
            "load_skill".to_string(),
            load_skill_tool(skill_registry.clone()),
        ),
    ]);

    let mut state = LoopState::new(client.clone(), tools, skill_registry.clone());

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

async fn agent_loop(state: &mut LoopState) -> Result<()> {
    let system = format!(
        r#"You are a coding agent at {}.
Use load_skill when a task needs specialized instructions before you act.

Skills available:
    {}
"#,
        std::env::current_dir()?.display(),
        state.skill_registry.describe_available()
    );
    loop {
        let request = CreateMessageParams::new(RequiredMessageParams {
            model: MODEL.to_string(),
            messages: state.context.clone(),
            max_tokens: 8000,
        })
        .with_system(&system)
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
