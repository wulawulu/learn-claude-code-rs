use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;

use inquire::{Select, Text};
use s19_mcp_plugin::{
    LoopState, extract_text, get_llm_client, load_mcp_router,
    permission::{PermissionManager, PermissionMode},
    tool::toolset,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;

    let tools = toolset();
    let mcp_router = load_mcp_router().await?;

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
    println!(
        "[Tool pool: {} native, {} MCP]",
        tools.len(),
        mcp_router.all_tools().len()
    );

    let mut state = LoopState::new(client.clone(), tools, mcp_router, permission_manager);

    loop {
        let query = Text::new("--- How can I help you?")
            .prompt()
            .context("An error happened or user cancelled the input.")?;

        let query = query.trim();

        if query.is_empty() || matches!(query, "q" | "exit" | "exit()") {
            break;
        }

        if query == "/rules" {
            for (index, rule) in state.permission_manager.rules().iter().enumerate() {
                println!("  {index}: {rule}");
            }
            continue;
        }

        if query == "/tools" {
            let mut specs = state.all_tool_specs();
            specs.sort_by(|a, b| a.name.cmp(&b.name));
            for spec in specs {
                println!("  {}", spec.name);
            }
            continue;
        }

        if query == "/mcp" {
            for (server, tool_count) in state.mcp_router.server_summaries() {
                println!("  {server}: {tool_count} tools");
            }
            continue;
        }

        if query.starts_with("/mode") {
            state
                .handle_mode_command(query)
                .context("failed to switch permission mode")?;
            continue;
        }

        state
            .context
            .push(Message::new_text(User, query.to_string()));

        state.agent_loop().await?;

        let Some(final_content) = state.context.last() else {
            continue;
        };
        println!(
            "--- Final response:\n{}",
            extract_text(&final_content.content)
        );
    }

    state.mcp_router.disconnect_all().await;

    Ok(())
}
