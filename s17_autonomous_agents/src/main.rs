use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;

use inquire::Text;
use s17_autonomous_agents::{
    LoopState, extract_text, get_llm_client,
    task::SharedTaskManager,
    team::{SharedTeammateManager, TEAM_DIR_NAME},
    tool::leader_tools,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;
    let team_dir = std::env::current_dir()?.join(TEAM_DIR_NAME);
    let manager = SharedTeammateManager::new(&team_dir)?;
    manager.register_mailbox("lead");
    let system_prompt = format!(
        "You are a team lead at {}. Manage teammates with shutdown and plan approval protocols.",
        std::env::current_dir()?.display()
    );

    let tasks = SharedTaskManager::new(std::env::current_dir()?.join(".tasks"))?;

    let tools = leader_tools(manager.clone(), tasks);

    let mut state: LoopState = LoopState::new(
        client,
        tools,
        manager.clone(),
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
                serde_json::to_string_pretty(&manager.read_inbox("lead")?)?
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
