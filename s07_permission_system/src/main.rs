use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;

use inquire::{Select, Text};
use s07_permission_system::{
    LoopState, extract_text, get_llm_client,
    permission::{PermissionManager, PermissionMode},
    tool::toolset,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;

    let tools = toolset();

    let mode = Select::new(
        "Permission mode:",
        vec![
            PermissionMode::Default,
            PermissionMode::Plan,
            PermissionMode::Auto,
        ],
    )
    .prompt()
    .context("An error happened or user cancelled the input.")?;

    let permission_manager = PermissionManager::try_new(mode)?;
    println!("[Permission mode: {}]", permission_manager.mode());

    let mut state = LoopState::new(client.clone(), tools, permission_manager);

    loop {
        let query = Text::new("--- How can I help you?")
            .prompt()
            .context("An error happened or user cancelled the input.")?;

        //break out of the loop if the user enters exit()
        if query.trim() == "exit()" {
            break;
        }

        if query.trim() == "/rules" {
            for (index, rule) in state.permission_manager.rules().iter().enumerate() {
                println!("  {index}: {rule}");
            }
            continue;
        }

        if query.trim().starts_with("/mode") {
            state
                .handle_mode_command(&query)
                .context("failed to switch permission mode")?;
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
