use std::sync::Arc;

use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;
use inquire::{Select, Text};

use sfull::{
    Agent, AgentSystemPrompt,
    background::SharedBackgroundManager,
    cron::{CronScheduler, SharedCronScheduler},
    extract_text, get_llm_client,
    mcp::load_mcp_router,
    memory::get_memory_manager,
    permission::{PermissionManager, PermissionMode},
    skill::get_skill_registry,
    store::StoreRoot,
    task::{SharedTaskManager, TaskManager},
    team::{SharedTeammateManager, TeammateManager},
    tool::{ToolContext, toolset},
    worktree::{SharedWorktreeManager, WorktreeManager},
};

const SKILLS_DIR: &str = "skills";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;

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

    let skills_dir = std::env::current_dir()?.join(SKILLS_DIR);
    let skill_registry = Arc::new(get_skill_registry(skills_dir)?);
    let work_dir = std::env::current_dir()?;
    let store_root = StoreRoot::new(work_dir.join(".claude"))?;
    let task_manager = SharedTaskManager::new(TaskManager::new(&store_root)?);
    let background_manager = SharedBackgroundManager::new(&store_root)?;
    let cron_scheduler = SharedCronScheduler::new(CronScheduler::new(&store_root)?);
    let teammate_manager = SharedTeammateManager::new(TeammateManager::new(&store_root)?);
    let worktree_manager =
        SharedWorktreeManager::new(WorktreeManager::new(&store_root, work_dir.clone())?);
    let memory_manager = Arc::new(std::sync::Mutex::new(get_memory_manager(
        work_dir.join(".claude/memory"),
    )?));
    let mcp_router = load_mcp_router().await?;

    let tools = toolset();
    let tool_context = ToolContext {
        skill_registry: skill_registry.clone(),
        memory_manager,
        work_dir,
        task_manager,
        background_manager,
        cron_scheduler,
        teammate_manager,
        worktree_manager,
    };

    let mut agent = Agent::new(
        client.clone(),
        tool_context,
        tools,
        mcp_router,
        permission_manager,
        AgentSystemPrompt::Dynamic,
    );

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
