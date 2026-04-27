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

pub trait PreToolUseFn:
    for<'a> Fn(
        &'a LoopState,
        &'a mut ToolUse,
    ) -> Pin<Box<dyn Future<Output = Result<HookControl>> + Send + 'a>>
    + Send
    + Sync
{
}

pub trait PostToolUseFn:
    for<'tool> Fn(
        &'tool LoopState,
        &'tool ToolUse,
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
            &'tool mut ToolUse,
        ) -> Pin<Box<dyn Future<Output = Result<HookControl>> + Send + 'tool>>
        + Send
        + Sync
{
}

impl<F> PostToolUseFn for F where
    F: for<'tool> Fn(
            &'tool LoopState,
            &'tool ToolUse,
            &'tool mut ToolResult,
        ) -> Pin<Box<dyn Future<Output = Result<HookControl>> + Send + 'tool>>
        + Send
        + Sync
{
}

#[derive(strum_macros::EnumDiscriminants, strum_macros::Display)]
#[strum_discriminants(name(HookTypes), derive(strum_macros::Display))]
pub enum Hook {
    SessionStart(Box<dyn SessionStartFn>),
    PreToolUse(Box<dyn PreToolUseFn>),
    PostToolUse(Box<dyn PostToolUseFn>),
}

#[macro_export]
macro_rules! invoke_hooks {
    ($hook_type:ident, $self_expr:expr $(, $arg:expr)* ) => {{
        let mut control = $crate::hook::HookControl::Continue;

        for hook in $self_expr.hooks_by_type($crate::hook::HookTypes::$hook_type) {
            if let $crate::hook::Hook::$hook_type(hook_fn) = hook {
                match hook_fn($self_expr $(, $arg)*).await? {
                    $crate::hook::HookControl::Continue => {}
                    $crate::hook::HookControl::Block(reason) => {
                        control = $crate::hook::HookControl::Block(reason);
                        break;
                    }
                }
            }
        }

        anyhow::Ok(control)
    }};
}
