use std::collections::HashMap;

use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;

use inquire::Text;
use s12_task_system::{
    LoopState, extract_text, get_llm_client,
    task::SharedTaskManager,
    tool::{
        bash_tool, edit_file_tool, read_file_tool, task_create_tool, task_get_tool, task_list_tool,
        task_update_tool, write_file_tool,
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;
    let tasks = SharedTaskManager::new(std::env::current_dir()?.join(".tasks"))?;

    let tools = HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("edit_file".to_string(), edit_file_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("task_create".to_string(), task_create_tool(tasks.clone())),
        ("task_get".to_string(), task_get_tool(tasks.clone())),
        ("task_list".to_string(), task_list_tool(tasks.clone())),
        ("task_update".to_string(), task_update_tool(tasks.clone())),
        ("write_file".to_string(), write_file_tool()),
    ]);

    let mut state: LoopState = LoopState::new(client.clone(), tools);
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
