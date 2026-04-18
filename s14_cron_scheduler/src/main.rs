use std::collections::HashMap;

use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::Context;

use inquire::Text;
use s14_cron_scheduler::{
    LoopState,
    cron::SharedCronScheduler,
    extract_text, get_llm_client,
    tool::{
        bash_tool, cron_create_tool, cron_delete_tool, cron_list_tool, edit_file_tool,
        read_file_tool, write_file_tool,
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;
    let scheduler = SharedCronScheduler::new(std::env::current_dir()?)?;
    let loaded = scheduler.start()?;

    println!("[Cron scheduler running. Background checks every second.]");
    println!("[Commands: /cron to list tasks, /test to fire a test notification]");
    if loaded > 0 {
        println!("[Cron] Loaded {loaded} scheduled tasks");
    }

    let tools = HashMap::from([
        ("bash".to_string(), bash_tool()),
        (
            "cron_create".to_string(),
            cron_create_tool(scheduler.clone()),
        ),
        (
            "cron_delete".to_string(),
            cron_delete_tool(scheduler.clone()),
        ),
        ("cron_list".to_string(), cron_list_tool(scheduler.clone())),
        ("edit_file".to_string(), edit_file_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("write_file".to_string(), write_file_tool()),
    ]);

    let mut state: LoopState = LoopState::new(client.clone(), tools, scheduler.clone());
    loop {
        let query = Text::new("--- How can I help you?")
            .prompt()
            .context("An error happened or user cancelled the input.")?;

        //break out of the loop if the user enters exit()
        if query.trim() == "exit()" {
            scheduler.stop().await;
            break;
        }

        if query.trim() == "/cron" {
            println!("{}", scheduler.list_tasks()?);
            continue;
        }

        if query.trim() == "/test" {
            scheduler.enqueue_test_notification();
            println!("[Test notification enqueued. It will be injected on your next message.]");
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
