use std::collections::HashMap;

use anthropic_ai_sdk::types::message::{
    CreateMessageParams, Message, MessageClient, RequiredMessageParams,
    Role::{self, User},
    StopReason,
};
use anyhow::{Context, Result};
use inquire::Text;

use s06_context_compact::{
    LoopState, MODEL,
    compact::{estimate_context_size, micro_compact},
    extract_text, get_llm_client,
    tool::{bash_tool, compact_tool, edit_file_tool, read_file_tool, write_file_tool},
};

const CONTEXT_LIMIT: usize = 50000;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;

    let tools = HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("compact".to_string(), compact_tool()),
        ("edit_file".to_string(), edit_file_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("write_file".to_string(), write_file_tool()),
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

async fn agent_loop(state: &mut LoopState) -> Result<()> {
    let system = format!(
        r#"You are a coding agent at {}.
Keep working step by step, and use compact if the conversation gets too long.
"#,
        std::env::current_dir()?.display(),
    );
    loop {
        micro_compact(&mut state.context);

        if estimate_context_size(&state.context) > CONTEXT_LIMIT {
            println!("[auto compact]");
            state.compact_history(None).await?;
        }

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

        state.execute_tool_call(&response.content).await?;
    }
}
