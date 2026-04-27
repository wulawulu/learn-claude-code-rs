# sfull

`sfull` 是前面章节机制的整合版本。它以 `s20_tool_refactor` 的工具系统为基线，把 `s05` 到 `s20` 中已经实现过的能力收敛到一个 Agent runtime 中，同时对有明确增删改查语义的 domain 状态引入统一的 Store 层。

这版的核心目标不是重新设计所有机制，而是把已有章节的实现放到同一个结构里，让代码边界更清楚：

- `Agent` 负责运行循环、上下文、权限、hook、恢复、system prompt 和 MCP tool 路由。
- `ToolRouter` 负责本地工具注册和强类型 schema 生成。
- `ToolContext` 只注入工具需要的共享 domain manager。
- `TaskManager`、`TeammateManager`、`CronScheduler` 等 manager 表达领域动作。
- `Store<T>` / `CollectionStore<T>` 只在 manager 内部处理文件读写。

## 基线

`sfull` 延续 `s20_tool_refactor` 的结构：

- 每个本地工具实现 `Tool` trait。
- tool input 使用具体 Rust 类型，并通过 `schemars` 生成 JSON schema。
- `ToolRouter` 以工具名分发调用。
- 同一个 `ToolContext` 注入所有工具共享依赖。
- 工具按 domain 分文件组织，例如 `tool/task.rs`、`tool/cron.rs`、`tool/team.rs`。

相比 `s05` 以前的手写 tool schema 和 `HashMap<String, Box<dyn Tool>>`，`sfull` 保留了更清晰的 router + typed input 结构。

## 已整合机制

当前 `sfull` 整合了这些章节机制：

- `s05_skill_loading`：扫描并加载 `skills/*/SKILL.md`。
- `s06_context_compact`：自动 compact、手动 `compact` 工具、micro compact、大输出落盘预览、transcript 写入。
- `s07_permission_system`：启动时选择 `Default`、`Plan`、`Auto` 权限模式；支持 allow、ask、deny。
- `s08_hook_system`：保留 `SessionStart`、`PreToolUse`、`PostToolUse` hook 形态。
- `s09_memory_system`：保存和加载项目记忆、用户偏好、反馈、事实和引用。
- `s10_system_prompt`：动态拼装 system prompt，包含技能、memory、`CLAUDE.md` 和当前上下文。
- `s11_error_recovery`：处理 prompt too long、临时传输错误和 max tokens 截断。
- `s12_task_system`：任务创建、查询、更新、列表、依赖和 owner。
- `s13_background_tasks`：后台命令执行和状态查询。
- `s14_cron_scheduler`：定时任务创建、删除和列表。
- `s15_agent_teams` / `s16_team_protocols`：teammate 管理、消息、广播、plan approval 和 shutdown protocol。
- `s18_worktree_task_isolation`：worktree 创建、列表、状态、执行命令和事件记录。
- `s04_subagent`：通过 `task` 工具创建 fresh-context subagent。
- `s19_mcp_plugin`：读取 plugin manifest，连接 MCP server，把 MCP tools 暴露给模型。
- `s20_tool_refactor`：工具按 router 注册，强类型输入和 schema 生成。

## 启动流程

入口在 `src/main.rs`。启动顺序是：

1. 创建 LLM client。
2. 让用户选择 `PermissionMode`。
3. 初始化技能注册表。
4. 创建 `.claude` 下的 `StoreRoot`。
5. 构造各个 domain manager。
6. 初始化 memory manager。
7. 读取 `.claude-plugin/plugin.json` 并连接 MCP server。
8. 构造 `ToolContext`。
9. 通过统一的 `Agent::new(...)` 创建 Agent。

`Agent::new` 不再通过 `with_*` 追加配置。当前构造参数显式包含：

```rust
pub fn new(
    client: AnthropicClient,
    tool_context: ToolContext,
    tools: ToolRouter,
    mcp_router: MCPToolRouter,
    permission_manager: PermissionManager,
    system_prompt: AgentSystemPrompt,
) -> Self
```

主 Agent 使用 `AgentSystemPrompt::Dynamic`，subagent 使用 `AgentSystemPrompt::Static(...)`。

## Agent 运行循环

核心循环在 `src/lib.rs`：

1. 每轮先做 `micro_compact`。
2. 如果上下文超过限制，触发自动 compact。
3. 构造 message request，并合并本地工具和 MCP 工具 schema。
4. 调用模型。
5. 如果模型输出被截断，注入 continuation message 重试。
6. 如果模型请求 tool use：
   - 执行 `PreToolUse` hook。
   - 通过 `PermissionManager` 做 allow、ask、deny。
   - 本地工具交给 `ToolRouter`。
   - `mcp__...` 工具交给 `MCPToolRouter`。
   - 执行 `PostToolUse` hook。
   - tool result 回填到上下文。
7. 如果调用了 `compact` 工具，执行手动 compact。

## 权限模式

启动时必须选择权限模式，不再隐式使用 default：

- `Default`：只读能力直接允许，写入能力询问用户，高危能力询问用户。
- `Plan`：只读能力允许，写入能力拒绝。
- `Auto`：只读和非高危写入自动允许，高危能力询问用户。

权限判断在 tool 执行前发生，因此 `PermissionManager` 属于 `AgentRuntime`，不放进 `ToolContext`。

`bash` 会做保守分类：

- `ls`、`pwd`、`cat`、`head`、`tail`、`wc`、`rg`、`grep` 等简单只读命令归为 read。
- `git status`、`git diff`、`git log`、`git show`、`git branch` 归为 read。
- 带 shell 组合符或重定向的命令不会自动归为 read。
- `sudo`、`rm -rf`、`shutdown`、`reboot` 等归为 high risk。

## ToolContext

`ToolContext` 是工具层的依赖注入对象：

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

`ToolContext` 的字段不是 `Option`，工具可以直接通过 `ctx.task_manager`、`ctx.cron_scheduler` 等字段调用。

`SharedTaskManager`、`SharedTeammateManager`、`SharedCronScheduler`、`SharedWorktreeManager` 等共享封装实现了 `Deref`，保留共享所有权和锁语义，同时让调用侧更直接。

## Store 和 Domain Manager

是的，当前设计就是：有明确 CRUD 状态的 domain manager 不直接处理文件细节，而是通过内部的 Store 处理落盘。

边界如下：

- `StoreRoot`：表示 `.claude` 状态根目录，负责安全地解析相对路径。
- `Store<T>`：表示单个 typed JSON 文件，也支持 JSONL append/read_all。
- `CollectionStore<T>`：表示一组 typed JSON 文件或 JSONL 文件集合。
- Domain manager：持有 store，暴露领域方法。
- Tool：调用 domain manager，不直接拼领域状态路径。

例如 task：

```rust
pub struct TaskManager {
    tasks: CollectionStore<TaskRecord>,
    index: Store<TaskIndex>,
}
```

调用方只应该使用：

```rust
task_manager.create(...)
task_manager.update(...)
task_manager.list(...)
task_manager.get(...)
```

调用方不应该知道 `tasks/index.json`、`task_1.json` 或 `CollectionStore<TaskRecord>` 的存在。

team 也是同样的边界：

```rust
pub struct TeammateManager {
    config: Store<TeamConfig>,
    inboxes: CollectionStore<InboxMessage>,
}
```

调用方只使用：

```rust
teammate_manager.spawn_teammate(...)
teammate_manager.send_message(...)
teammate_manager.read_inbox(...)
teammate_manager.broadcast(...)
```

Store 不表达业务规则。比如任务完成后清理依赖、队友名字不能重复、cron id 如何递增，这些逻辑都留在 manager 中。

## 哪些地方使用 Store

当前已经通过 Store 管理的 domain 状态：

- `TaskManager`
  - `tasks/index.json`
  - `tasks/<task>.json`
- `BackgroundManager`
  - `background/tasks/<id>.json`
- `CronScheduler`
  - `cron/scheduled_tasks.json`
- `TeammateManager`
  - `team/config.json`
  - `team/inbox/<owner>.json`
- `WorktreeManager`
  - `worktrees/index.json`

这些 manager 内部会调用 `Store<T>` 或 `CollectionStore<T>`，tool 和 Agent loop 不直接碰这些文件。

## 哪些地方不使用 Store

Store 只用于明确有领域 CRUD 语义的状态，不用于所有文件操作。

当前不强行纳入 Store 的场景：

- `read_file`、`write_file`、`edit_file`：用户显式操作 workspace 文件，文件本身就是业务对象。
- skill loading：读取 `skills/*/SKILL.md`，这是技能文件系统结构，不是 domain CRUD。
- memory markdown：memory 以 markdown/frontmatter 为业务格式，保持自己的 manager。
- transcript 和大输出落盘：属于 compact/recovery 辅助文件，不是独立 domain。
- MCP plugin manifest：读取 `.claude-plugin/plugin.json`，这是外部插件配置。
- git worktree 目录本身：由 `git worktree` 管理，Store 只保存索引。

这个边界很重要：Store 是 domain manager 的持久化成员，不是通用文件 API。

## 状态目录

默认状态根目录是当前工作区下的 `.claude`：

```text
.claude/
  background/
    tasks/
      <id>.json
  cron/
    scheduled_tasks.json
  memory/
    MEMORY.md
    *.md
  tasks/
    index.json
    task_<id>.json
  team/
    config.json
    inbox/
      <owner>.json
  worktrees/
    index.json
```

`StoreRoot` 要求传入相对路径，拒绝逃出 `.claude` root 的路径。JSON 写入使用 pretty format 并在末尾加换行；JSONL append 每条记录一行。

## 工具列表

本地工具由 `toolset()` 注册：

- 基础工具：`add`、`bash`、`read_file`、`write_file`、`edit_file`
- skill：`load_skill`
- memory：`save_memory`
- compact：`compact`
- subagent：`task`
- task domain：`task_create`、`task_get`、`task_list`、`task_update`
- background：`background_run`、`background_check`
- cron：`cron_create`、`cron_delete`、`cron_list`
- team：`spawn_teammate`、`list_teammates`、`send_message`、`broadcast`、`read_inbox`、`plan_approval`、`shutdown_request`、`shutdown_response`
- worktree：`worktree_create`、`worktree_list`、`worktree_status`、`worktree_run`、`worktree_events`

subagent 使用单独的 `subagent_toolset()`，只开放：

- `bash`
- `read_file`
- `write_file`
- `edit_file`

这样 subagent 具备独立探索和修改能力，但不会递归创建新的 task/team/cron/worktree 复杂控制面。

## MCP

MCP 逻辑在 `src/mcp.rs`。

启动时会扫描当前工作目录下：

```text
.claude-plugin/plugin.json
```

manifest 中的 `mcpServers` 会被启动并连接。连接成功后，server tools 会转换为 Agent 可见的 tool schema，名称格式为：

```text
mcp__<plugin>__<server>__<tool>
```

执行时：

- 本地工具走 `ToolRouter`。
- 名字以 `mcp__` 开头的工具走 `MCPToolRouter`。

MCP router 属于 Agent，不属于 `ToolContext`。这是因为 MCP tool schema 要和本地 tool schema 一起暴露给模型，而执行也发生在 Agent 的 tool dispatch 阶段。

## Hook

hook 定义在 `src/hook.rs`，当前保留三类：

- `SessionStart`
- `PreToolUse`
- `PostToolUse`

`PreToolUse` 可以修改 tool input 或阻断工具调用。`PostToolUse` 可以修改 tool result 或阻断结果返回。

当前 Agent loop 在工具调用前后执行 `PreToolUse` / `PostToolUse`。`SessionStart` 的类型和注册方法保留，方便后续接入启动事件。

## System Prompt

system prompt 由 `src/prompt.rs` 和 `src/system_prompt_template.md` 生成。

动态 prompt 包含：

- Agent 角色和工作目录。
- 行为约束。
- 可用技能列表。
- memory 内容。
- `CLAUDE.md` 内容。
- 当前时间、git branch、最近文件等 dynamic context。
- memory 使用指引。

主 Agent 使用动态 prompt。subagent 使用静态 prompt，强调它是 fresh-context coding subagent，并在完成后总结发现。

## 与前面章节的关系

`sfull` 的实现大部分直接复用前面章节：

- Agent loop、tool calling、subagent、skill、compact、permission、hook、memory、system prompt、recovery、MCP 都按旧章节风格整合。
- 工具系统使用 `s20_tool_refactor` 的更清晰结构。
- Store 是主要新增抽象，用来收敛 task/team/cron/background/worktree 这类 domain 状态的文件 CRUD。

因此阅读顺序建议是：

1. 先看 `src/main.rs` 理解初始化。
2. 再看 `src/lib.rs` 理解 Agent loop。
3. 再看 `src/tool/mod.rs` 理解工具注册和 `ToolContext`。
4. 再看 `src/store.rs` 和具体 manager，理解 Store 如何被封装在 domain 内。
5. 最后按需看 `permission.rs`、`mcp.rs`、`prompt.rs`、`compact.rs`。

## 运行和验证

运行前需要设置：

```bash
ANTHROPIC_API_KEY=...
ANTHROPIC_BASE_URL=...
```

启动：

```bash
cargo run -p sfull
```

验证：

```bash
cargo check -p sfull
cargo test -p sfull
```
