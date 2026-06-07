use std::collections::HashMap;

use anyhow::{Context, Result};
use inquire::Text;
use s21_llm_trait::{
    ChatCompletion, ChatCompletionRequest, ChatMessage, ToolCall,
    providers::{AnthropicChatCompletion, OpenAIChatCompletion},
    tool::{Tool, toolset},
};

const SYSTEM: &str = r#"You are a coding agent.
Use bash to inspect and change the workspace. Act first, then report clearly.
"#;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let llm = llm_from_env()?;
    let mut state = LoopState {
        llm,
        context: vec![ChatMessage::System(SYSTEM.to_string())],
        tools: toolset(),
    };

    loop {
        let query = Text::new("--- How can I help you?")
            .prompt()
            .context("An error happened or user cancelled the input.")?;
        if query.trim() == "exit()" {
            break;
        }

        state.context.push(ChatMessage::User(query));
        let final_response = agent_loop(&mut state).await?;
        println!("--- Final response:\n{final_response}");
    }

    Ok(())
}

fn llm_from_env() -> Result<Box<dyn ChatCompletion>> {
    match std::env::var("LLM_PROVIDER")
        .unwrap_or_else(|_| "anthropic".to_string())
        .to_lowercase()
        .as_str()
    {
        "openai" => Ok(Box::new(OpenAIChatCompletion::new(
            env("OPENAI_API_KEY")?,
            std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
            env("OPENAI_MODEL")?,
        ))),
        "anthropic" => Ok(Box::new(AnthropicChatCompletion::new(
            env("ANTHROPIC_API_KEY")?,
            std::env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com/v1".to_string()),
            env("ANTHROPIC_MODEL")?,
        )?)),
        provider => anyhow::bail!("Unsupported LLM_PROVIDER: {provider}"),
    }
}

fn env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} is not set"))
}

struct LoopState {
    llm: Box<dyn ChatCompletion>,
    context: Vec<ChatMessage>,
    tools: HashMap<String, Box<dyn Tool>>,
}

async fn agent_loop(state: &mut LoopState) -> Result<String> {
    loop {
        let request = ChatCompletionRequest {
            messages: state.context.clone(),
            tools: state.tools.values().map(|tool| tool.tool_spec()).collect(),
        };
        let response = state.llm.complete(&request).await?;
        let final_content = response.content.clone().unwrap_or_default();
        let tool_calls = response.tool_calls.clone();
        state.context.push(response.into_message());

        if tool_calls.is_empty() {
            return Ok(final_content);
        }

        for call in tool_calls {
            let output = execute_tool_call(&state.tools, &call).await;
            state.context.push(ChatMessage::Tool {
                tool_call_id: call.id,
                content: output,
            });
        }
    }
}

async fn execute_tool_call(tools: &HashMap<String, Box<dyn Tool>>, call: &ToolCall) -> String {
    let Some(tool) = tools.get(&call.name) else {
        return format!("Unknown tool: {}", call.name);
    };

    match tool.invoke(&call.arguments).await {
        Ok(output) => {
            println!(
                "Command:{}\n arg:{}\n output:\n{}\n",
                call.name, call.arguments, output
            );
            output
        }
        Err(error) => {
            println!("Error invoking tool {}: {}", call.name, error);
            format!("Error invoking tool {}: {}", call.name, error)
        }
    }
}
