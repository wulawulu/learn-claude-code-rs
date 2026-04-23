use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;

use inquire::Text;
use s18_worktree_task_isolation::{
    LoopState, canonical_work_dir, detect_repo_root, extract_text, get_llm_client,
    task::SharedTaskManager, tool::toolset, worktree::SharedWorktreeManager,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = detect_repo_root(&cwd).unwrap_or(cwd);
    let work_dir = canonical_work_dir(&repo_root)?;
    let client = get_llm_client()?;
    let tasks = SharedTaskManager::new(repo_root.join(".tasks"))?;
    let worktrees = SharedWorktreeManager::new(&repo_root, tasks.clone())?;

    let tools = toolset(tasks.clone(), worktrees.clone(), work_dir.clone());

    let system = format!(
        "You are a coding agent at {}. Tasks track what to do. Worktrees track where to do it. For parallel or risky implementation, create or inspect a task, allocate a worktree lane, and delegate isolated implementation to a subagent bound to that worktree.",
        work_dir.display(),
    );

    if !worktrees.git_available() {
        println!("Note: not in a git repository. worktree_* tools will return errors.");
    }

    let mut state: LoopState = LoopState::new(client.clone(), tools, system, usize::MAX);
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
