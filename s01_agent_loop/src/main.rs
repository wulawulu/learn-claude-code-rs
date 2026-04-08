use async_openai::{
    Client,
    traits::RequestOptionsBuilder,
    types::chat::{
        ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
        CreateChatCompletionRequestArgs,
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    // Create client
    let client = Client::new();

    let request = CreateChatCompletionRequestArgs::default()
        .max_tokens(512u32)
        .model("deepseek-chat")
        .messages([
            // Can also use ChatCompletionRequest<Role>MessageArgs for builder pattern
            ChatCompletionRequestSystemMessage::from("You are a helpful assistant.").into(),
            ChatCompletionRequestUserMessage::from("What is the capital of France?").into(),
        ])
        .build()?;

    println!("{}", serde_json::to_string(&request).unwrap());

    let response = client
        .chat()
        .query(&vec![("limit", 10)])?
        .create(request)
        .await?;

    println!("\nResponse:\n");
    for choice in response.choices {
        println!(
            "{}: Role: {}  Content: {:?}",
            choice.index, choice.message.role, choice.message.content
        );
    }

    Ok(())
}
