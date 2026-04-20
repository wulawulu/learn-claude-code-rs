use anthropic_ai_sdk::types::message::{
    CreateMessageParams, Message, MessageClient, RequiredMessageParams,
    Role::{self, User},
    StopReason,
};
use anyhow::{Context, Result};
use inquire::Text;

use s04_subagent::{LoopState, MODEL, extract_text, get_llm_client, tool::agent_tools};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;

    let tools = agent_tools();

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

async fn agent_loop(state: &mut LoopState) -> Result<()> {
    let system = format!(
        "You are a coding agent at {}. Use the task tool to delegate exploration or subtasks.",
        std::env::current_dir()?.display()
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
