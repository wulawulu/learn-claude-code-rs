use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;
use inquire::Text;

use s04_subagent::{LoopState, extract_text, get_llm_client, tool::agent_tools};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;
    let system_prompt = format!(
        "You are a coding agent at {}. Use the task tool to delegate exploration or subtasks.",
        std::env::current_dir()?.display()
    );
    let tools = agent_tools();
    let mut state = LoopState::new(client.clone(), tools, system_prompt, usize::MAX);

    loop {
        let query = Text::new("--- How can I help you?")
            .prompt()
            .context("An error happened or user cancelled the input.")?;

        //break out of the loop if the user enters exit()
        if query.trim() == "exit()" {
            break;
        }
        state.context.push(Message::new_text(User, query));

        state.agent_loop().await?;

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
