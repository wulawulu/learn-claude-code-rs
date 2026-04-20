use std::sync::{Arc, Mutex};

use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;

use inquire::Text;
use s09_memory_system::{
    LoopState, extract_text, get_llm_client, memory::get_memory_manager, tool::toolset,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;
    let memory_manager = Arc::new(Mutex::new(get_memory_manager(
        std::env::current_dir()?.join(".memory"),
    )?));

    let tools = toolset(memory_manager.clone());

    let mut state: LoopState = LoopState::new(client.clone(), tools, memory_manager.clone());
    loop {
        let query = Text::new("--- How can I help you?")
            .prompt()
            .context("An error happened or user cancelled the input.")?;

        //break out of the loop if the user enters exit()
        if query.trim() == "exit()" {
            break;
        }

        if query.trim() == "/memories" {
            let memory_manager = memory_manager
                .lock()
                .map_err(|_| anyhow::anyhow!("memory manager lock poisoned"))?;
            println!("{}", memory_manager.describe_memories());
            continue;
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
