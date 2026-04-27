# sfull: Complete Agent Harness

`sfull` is the integrated version of the previous chapters. It brings the minimal agent loop, tools, skills, context compaction, permissions, hooks, memory, tasks, background jobs, cron scheduling, team collaboration, worktrees, MCP, and tool routing into one Rust agent runtime.

This chapter is not about adding one more feature. It answers an engineering question:

```text
When an agent harness has many capabilities, how should loop, tools, state, permissions, and external plugins be organized?
```

## Run

Configure `.env`:

```bash
ANTHROPIC_API_KEY=your_api_key
ANTHROPIC_BASE_URL=your_anthropic_compatible_base_url
```

Run:

```bash
cargo run -p sfull
```

Choose a permission mode at startup:

```text
Default
Plan
Auto
```

Exit:

```text
exit()
```

## Goals

- Integrate the previous chapter features into one runnable agent.
- Use `Agent` as the main runtime boundary.
- Use `ToolRouter` for native tools.
- Use `ToolContext` to inject shared domain managers into tools.
- Use `Store<T>` and `CollectionStore<T>` for durable domain state.
- Let native tools and MCP tools share one tool-use loop.
- Run permission checks and hooks before and after tools.
- Recover from long context, truncated output, and transient errors.

## Code Layout

```text
sfull/
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── store.rs
│   ├── prompt.rs
│   ├── system_prompt_template.md
│   ├── permission.rs
│   ├── hook.rs
│   ├── compact.rs
│   ├── recovery.rs
│   ├── memory.rs
│   ├── skill.rs
│   ├── task.rs
│   ├── background.rs
│   ├── cron.rs
│   ├── team.rs
│   ├── worktree.rs
│   ├── mcp.rs
│   └── tool/
└── sfull.md
```

Read [`src/main.rs`](./src/main.rs) first, then [`src/lib.rs`](./src/lib.rs), then the domain managers and tool modules.

## Startup Flow

```text
create LLM client
  -> choose PermissionMode
  -> scan skills/
  -> create .claude StoreRoot
  -> initialize task/background/cron/team/worktree managers
  -> initialize memory manager
  -> scan .claude-plugin/plugin.json and connect MCP servers
  -> build ToolContext
  -> build ToolRouter
  -> create Agent
  -> enter interactive loop
```

The root agent uses a dynamic system prompt. Subagents use static prompts and fresh context.

## Agent

Core structure:

```rust
pub struct Agent {
    pub runtime: AgentRuntime,
    pub tool_context: ToolContext,
    pub tools: ToolRouter,
    pub mcp_router: MCPToolRouter,
    pub hooks: Vec<Hook>,
    pub system_prompt: AgentSystemPrompt,
}
```

This separates:

- runtime state
- tool-accessible context
- native tool router
- MCP tool router
- hooks
- system prompt mode

Tools do not receive the whole `Agent`; they receive only `ToolContext`.

## Agent Loop

```text
micro compact
  -> auto compact if context is too large
  -> build model request
  -> merge native and MCP tool schemas
  -> call model
  -> recover from prompt-too-long / transient error / max tokens
  -> if no tool_use, finish
  -> execute tool_use
  -> return tool_result
  -> run manual compact if requested
  -> continue
```

This is still the s01 loop, with production-oriented boundaries around each stage.

## ToolRouter and ToolContext

`ToolRouter` registers native tools. `ToolContext` provides shared dependencies:

```rust
pub struct ToolContext {
    pub skill_registry: Arc<SkillRegistry>,
    pub memory_manager: Arc<Mutex<MemoryManager>>,
    pub work_dir: PathBuf,
    pub task_manager: SharedTaskManager,
    pub background_manager: SharedBackgroundManager,
    pub cron_scheduler: SharedCronScheduler,
    pub teammate_manager: SharedTeammateManager,
    pub worktree_manager: SharedWorktreeManager,
}
```

Runtime policy such as permissions, recovery, hooks, and model context stays outside `ToolContext`.

## Native Tools

The full toolset includes:

- base: `add`, `bash`, `read_file`, `write_file`, `edit_file`
- skill: `load_skill`
- memory: `save_memory`
- compact: `compact`
- subagent: `task`
- tasks: `task_create`, `task_get`, `task_list`, `task_update`
- background: `background_run`, `background_check`
- cron: `cron_create`, `cron_delete`, `cron_list`
- team: `spawn_teammate`, `list_teammates`, `send_message`, `broadcast`, `read_inbox`, `plan_approval`, `shutdown_request`, `shutdown_response`
- worktree: `worktree_create`, `worktree_list`, `worktree_status`, `worktree_run`, `worktree_events`

Subagents get a smaller toolset: `bash`, `read_file`, `write_file`, and `edit_file`.

## Store

[`src/store.rs`](./src/store.rs) defines:

- `StoreRoot`: the `.claude` state root.
- `Store<T>`: one typed JSON file or JSONL file.
- `CollectionStore<T>`: a collection of typed JSON files.

Store handles persistence. Domain managers handle business rules.

## State Directory

```text
.claude/
  background/
  cron/
  memory/
  tasks/
  team/
  worktrees/
```

This is the durable state for the harness.

## Domain Managers

Managers include:

- `TaskManager`
- `BackgroundManager`
- `CronScheduler`
- `TeammateManager`
- `WorktreeManager`

Tools call managers. They do not directly edit domain state files.

## Permission, Hooks, Compact, Recovery

Tool calls go through:

```text
PreToolUse hook
  -> PermissionManager
  -> ToolRouter or MCPToolRouter
  -> PostToolUse hook
  -> tool_result
```

The runtime can compact history, persist transcripts, store large command output, retry transient errors, and continue truncated model output.

## System Prompt, Skills, Memory

The dynamic system prompt includes:

- role and working directory
- behavior constraints
- available skill summaries
- memory content
- `CLAUDE.md`
- dynamic context
- memory guidance

Skills are loaded on demand with `load_skill`. Memory is saved with `save_memory` and loaded into future prompts.

## MCP

MCP servers are loaded from:

```text
.claude-plugin/plugin.json
```

Their tools are exposed as:

```text
mcp__<plugin>__<server>__<tool>
```

Native and MCP tools share the same permission and result path.

## Limits

- One shared `ToolContext` type.
- Team protocol is minimal and not a full autonomous worker runtime.
- Worktree support does not merge or rebase results.
- Store has no cross-process file locking.
- MCP only covers stdio tool calls.
- Hooks are not configuration-driven yet.

## Verify

```bash
cargo check -p sfull
cargo test -p sfull
cargo check --workspace
```
