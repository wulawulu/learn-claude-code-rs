use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::{Context, Ok};

use inquire::Text;
use s10_system_prompt::{
    LoopState, extract_text, get_llm_client,
    memory::get_memory_manager,
    skill::get_skill_registry,
    tool::{
        bash_tool, edit_file_tool, load_skill_tool, read_file_tool, save_memory_tool,
        write_file_tool,
    },
};
const SKILLS_DIR: &str = "skills";
const MEMORY_DIR: &str = ".memory";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;

    let skills_dir = std::env::current_dir()?.join(SKILLS_DIR);
    let skill_registry = Arc::new(get_skill_registry(skills_dir)?);
    let memory_manager = Arc::new(Mutex::new(get_memory_manager(
        std::env::current_dir()?.join(MEMORY_DIR),
    )?));
    let tools = HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("edit_file".to_string(), edit_file_tool()),
        (
            "load_skill".to_string(),
            load_skill_tool(skill_registry.clone()),
        ),
        ("read_file".to_string(), read_file_tool()),
        (
            "save_memory".to_string(),
            save_memory_tool(memory_manager.clone()),
        ),
        ("write_file".to_string(), write_file_tool()),
    ]);

    let mut state = LoopState::new(
        client.clone(),
        tools,
        skill_registry.clone(),
        memory_manager,
    );
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
