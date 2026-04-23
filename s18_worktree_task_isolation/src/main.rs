use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;

use inquire::Text;
use s18_worktree_task_isolation::{
    LoopState, extract_text, get_llm_client, task::SharedTaskManager, tool::toolset,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;
    let tasks = SharedTaskManager::new(std::env::current_dir()?.join(".tasks"))?;

    let tools = toolset(tasks.clone());

    let system = format!(
        "You are a coding agent at {}. Use task tools to plan and track work.",
        std::env::current_dir()?.display(),
    );

    let mut state: LoopState = LoopState::new(client.clone(), tools, system, usize::MAX);
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
