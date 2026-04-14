use anyhow::Result;
use std::pin::Pin;

use crate::LoopState;

#[derive(Debug)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookControl {
    Continue,
    Block(String),
}

pub trait SessionStartFn:
    for<'a> Fn(&'a LoopState) -> Pin<Box<dyn Future<Output = Result<HookControl>> + Send + 'a>>
    + Send
    + Sync
{
}

/// Hooks that are called before each tool
pub trait PreToolUseFn:
    for<'a> Fn(
        &'a LoopState,
        &mut ToolUse,
    ) -> Pin<Box<dyn Future<Output = Result<HookControl>> + Send + 'a>>
    + Send
    + Sync
{
}

/// Hooks that are called after each tool
pub trait PostToolUseFn:
    for<'tool> Fn(
        &'tool LoopState,
        &ToolUse,
        &'tool mut ToolResult,
    ) -> Pin<Box<dyn Future<Output = Result<HookControl>> + Send + 'tool>>
    + Send
    + Sync
{
}

impl<F> SessionStartFn for F where
    F: for<'a> Fn(&'a LoopState) -> Pin<Box<dyn Future<Output = Result<HookControl>> + Send + 'a>>
        + Send
        + Sync
{
}

impl<F> PreToolUseFn for F where
    F: for<'tool> Fn(
            &'tool LoopState,
            &mut ToolUse,
        ) -> Pin<Box<dyn Future<Output = Result<HookControl>> + Send + 'tool>>
        + Send
        + Sync
{
}

impl<F> PostToolUseFn for F where
    F: for<'tool> Fn(
            &'tool LoopState,
            &ToolUse,
            &'tool mut ToolResult,
        ) -> Pin<Box<dyn Future<Output = Result<HookControl>> + Send + 'tool>>
        + Send
        + Sync
{
}

/// Wrapper around the different types of hooks
#[derive(strum_macros::EnumDiscriminants, strum_macros::Display)]
#[strum_discriminants(name(HookTypes), derive(strum_macros::Display))]
pub enum Hook {
    /// Runs only once for the agent when it starts
    SessionStart(Box<dyn SessionStartFn>),
    /// Runs before every tool call, yielding a reference to the tool call
    PreToolUse(Box<dyn PreToolUseFn>),
    /// Runs after every tool call, yielding a reference to the tool call and a mutable result
    PostToolUse(Box<dyn PostToolUseFn>),
}
