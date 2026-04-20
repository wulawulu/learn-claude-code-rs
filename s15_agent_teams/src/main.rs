use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;

use inquire::Text;
use s15_agent_teams::{
    LoopState, extract_text, get_llm_client,
    team::{SharedMessageBus, SharedTeammateManager, TEAM_DIR_NAME},
    tool::leader_tools,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;
    let team_dir = std::env::current_dir()?.join(TEAM_DIR_NAME);
    let message_bus = SharedMessageBus::new(&team_dir)?;
    message_bus.register_mailbox("lead");
    let manager = SharedTeammateManager::new(&team_dir, message_bus.clone())?;
    let system_prompt = format!(
        "You are a team lead at {}. Spawn teammates and communicate via inboxes.",
        std::env::current_dir()?.display()
    );

    let tools = leader_tools(message_bus.clone(), manager.clone());

    let mut state: LoopState = LoopState::new(
        client,
        tools,
        message_bus.clone(),
        "lead",
        system_prompt,
        usize::MAX,
    );
    loop {
        let query = Text::new("--- How can I help you?")
            .prompt()
            .context("An error happened or user cancelled the input.")?;

        //break out of the loop if the user enters exit()
        if query.trim() == "exit()" {
            break;
        }

        if query.trim() == "/team" {
            println!("{}", manager.list_all()?);
            continue;
        }

        if query.trim() == "/inbox" {
            println!(
                "{}",
                serde_json::to_string_pretty(&message_bus.read_inbox("lead")?)?
            );
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
