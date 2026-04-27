use anyhow::Result;
use s20_tool_refactor_macros::tool;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::tool::ToolContext;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpawnTeammateInput {
    pub name: String,
    pub role: String,
}

#[tool(name = "spawn_teammate", description = "Create a named teammate.")]
pub async fn spawn_teammate(ctx: ToolContext, input: SpawnTeammateInput) -> Result<String> {
    ctx.teammate_manager.spawn_teammate(input.name, input.role)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTeammatesInput {}

#[tool(name = "list_teammates", description = "List teammates.")]
pub async fn list_teammates(ctx: ToolContext, _input: ListTeammatesInput) -> Result<String> {
    ctx.teammate_manager.list_teammates()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMessageInput {
    pub from: String,
    pub to: String,
    pub body: String,
}

#[tool(
    name = "send_message",
    description = "Send a message to a teammate inbox."
)]
pub async fn send_message(ctx: ToolContext, input: SendMessageInput) -> Result<String> {
    ctx.teammate_manager
        .send_message(input.from, input.to, input.body)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BroadcastInput {
    pub from: String,
    pub body: String,
}

#[tool(
    name = "broadcast",
    description = "Broadcast a message to all teammates."
)]
pub async fn broadcast(ctx: ToolContext, input: BroadcastInput) -> Result<String> {
    ctx.teammate_manager.broadcast(input.from, input.body)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadInboxInput {
    pub owner: String,
}

#[tool(name = "read_inbox", description = "Read a teammate inbox.")]
pub async fn read_inbox(ctx: ToolContext, input: ReadInboxInput) -> Result<String> {
    ctx.teammate_manager.read_inbox(&input.owner)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProtocolInput {
    pub from: String,
    pub to: String,
    pub body: String,
}

#[tool(
    name = "plan_approval",
    description = "Send a durable plan approval protocol message."
)]
pub async fn plan_approval(ctx: ToolContext, input: ProtocolInput) -> Result<String> {
    ctx.teammate_manager.protocol_request(
        input.from,
        input.to,
        "plan_approval".to_string(),
        input.body,
    )
}

#[tool(
    name = "shutdown_request",
    description = "Send a shutdown request protocol message."
)]
pub async fn shutdown_request(ctx: ToolContext, input: ProtocolInput) -> Result<String> {
    ctx.teammate_manager.protocol_request(
        input.from,
        input.to,
        "shutdown_request".to_string(),
        input.body,
    )
}

#[tool(
    name = "shutdown_response",
    description = "Send a shutdown response protocol message."
)]
pub async fn shutdown_response(ctx: ToolContext, input: ProtocolInput) -> Result<String> {
    ctx.teammate_manager.protocol_request(
        input.from,
        input.to,
        "shutdown_response".to_string(),
        input.body,
    )
}
