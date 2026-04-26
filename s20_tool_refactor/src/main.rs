use std::sync::Arc;

use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;
use inquire::Text;

use s20_tool_refactor::{
    Agent, extract_text, get_llm_client,
    skill::get_skill_registry,
    tool::{ToolContext, toolset},
};

const SKILLS_DIR: &str = "skills";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;

    let skills_dir = std::env::current_dir()?.join(SKILLS_DIR);
    let skill_registry = Arc::new(get_skill_registry(skills_dir)?);

    let tools = toolset();
    let tool_context = ToolContext {
        skill_registry: skill_registry.clone(),
        work_dir: std::env::current_dir()?,
    };

    let mut agent = Agent::new(client.clone(), tool_context, tools);

    loop {
        let query = Text::new("--- How can I help you?")
            .prompt()
            .context("An error happened or user cancelled the input.")?;

        //break out of the loop if the user enters exit()
        if query.trim() == "exit()" {
            break;
        }
        agent.runtime.context.push(Message::new_text(User, query));

        agent.agent_loop().await?;

        let Some(final_content) = agent.runtime.context.last() else {
            continue;
        };
        println!(
            "--- Final response:\n{}",
            extract_text(&final_content.content)
        );
    }

    Ok(())
}
