use std::sync::Arc;

use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;
use inquire::Text;

use s05_skill_loading::{
    LoopState, extract_text, get_llm_client, skill::get_skill_registry, tool::toolset,
};

const SKILLS_DIR: &str = "skills";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;

    let skills_dir = std::env::current_dir()?.join(SKILLS_DIR);
    let skill_registry = Arc::new(get_skill_registry(skills_dir)?);

    let tools = toolset(skill_registry.clone());

    let mut state = LoopState::new(client.clone(), tools, skill_registry.clone());

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
