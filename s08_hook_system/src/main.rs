use std::collections::HashMap;

use anthropic_ai_sdk::types::message::{Message, Role::User};
use anyhow::{Context, Ok};

use inquire::Text;
use s08_hook_system::{
    LoopState, extract_text, get_llm_client,
    hook::HookControl,
    invoke_hooks,
    tool::{bash_tool, edit_file_tool, read_file_tool, write_file_tool},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = get_llm_client()?;

    let tools = HashMap::from([
        ("bash".to_string(), bash_tool()),
        ("edit_file".to_string(), edit_file_tool()),
        ("read_file".to_string(), read_file_tool()),
        ("write_file".to_string(), write_file_tool()),
    ]);

    let mut state: LoopState = LoopState::new(client.clone(), tools);

    state.session_start(|_| {
        Box::pin(async {
            println!("--- Initializing...");
            Ok(HookControl::Continue)
        })
    });
    state.pre_tool(|_, tool_use| {
        println!("--- Before tool call: {:?}", tool_use);

        Box::pin(async move { Ok(HookControl::Continue) })
    });
    state.post_tool(|_, tool_use, tool_result| {
        println!(
            "--- After tool call: {:?}, result: {:?}",
            tool_use, tool_result
        );

        Box::pin(async move { Ok(HookControl::Continue) })
    });

    if let HookControl::Block(reason) = invoke_hooks!(SessionStart, &state)? {
        println!("--- Session blocked: {}", reason);
        return Ok(());
    }

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
