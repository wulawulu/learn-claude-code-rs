# sfull: 完整 Agent Harness

`sfull` 是前面章节的整合版本。它把最小 agent loop、工具系统、技能、上下文压缩、权限、hook、memory、任务、后台进程、cron、团队协作、worktree、MCP 和工具路由收敛到同一个 Rust agent runtime 中。

本章不是再引入一个新功能，而是回答一个工程问题：

```text
当 agent harness 的能力越来越多时，如何把 loop、tool、状态、权限和外部插件组织在一个清晰结构里？
```

## 运行方式

在仓库根目录配置 `.env`：

```bash
ANTHROPIC_API_KEY=your_api_key
ANTHROPIC_BASE_URL=your_anthropic_compatible_base_url
```

运行：

```bash
cargo run -p sfull
```

启动时会选择权限模式：

```text
Default
Plan
Auto
```

退出交互：

```text
exit()
```

## 本章目标

- 把前面章节的能力整合到一个可运行的 agent。
- 用 `Agent` 表达主运行时边界。
- 用 `ToolRouter` 管理本地工具。
- 用 `ToolContext` 给工具注入共享 domain manager。
- 用 `Store<T>` / `CollectionStore<T>` 收敛领域状态落盘。
- 让本地工具和 MCP 工具进入同一个 tool use 回路。
- 在工具执行前统一经过权限和 hook。
- 在上下文过长、输出截断、临时错误时做恢复。

## 代码结构

```text
sfull/
├── src/
│   ├── main.rs                   # 初始化和交互 CLI
│   ├── lib.rs                    # Agent runtime 和主 loop
│   ├── store.rs                  # StoreRoot / Store / CollectionStore
│   ├── prompt.rs                 # system prompt builder
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
│       ├── mod.rs                # Tool trait / ToolRouter / ToolContext
│       ├── bash.rs
│       ├── read_file.rs
│       ├── write_file.rs
│       ├── edit_file.rs
│       ├── load_skill.rs
│       ├── compact.rs
│       ├── memory.rs
│       ├── subagent.rs
│       ├── task.rs
│       ├── background.rs
│       ├── cron.rs
│       ├── team.rs
│       └── worktree.rs
└── sfull.md
```

建议先读 [`src/main.rs`](./src/main.rs)，再读 [`src/lib.rs`](./src/lib.rs)，最后按 domain 阅读各个 manager 和 tool。

## 启动流程

入口在 [`src/main.rs`](./src/main.rs)。启动顺序是：

```text
创建 LLM client
  -> 选择 PermissionMode
  -> 扫描 skills/
  -> 创建 .claude StoreRoot
  -> 初始化 task/background/cron/team/worktree manager
  -> 初始化 memory manager
  -> 扫描 .claude-plugin/plugin.json 并连接 MCP server
  -> 构造 ToolContext
  -> 构造 ToolRouter
  -> 创建 Agent
  -> 进入交互 loop
```

主 agent 使用动态 system prompt。subagent 使用静态 prompt，让它以 fresh-context coding subagent 的身份完成指定任务并返回总结。

## Agent

核心结构在 [`src/lib.rs`](./src/lib.rs)：

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

它把 agent 拆成几块：

- `AgentRuntime`：模型客户端、上下文、compact 状态、recovery 状态、权限管理。
- `ToolContext`：工具可访问的业务依赖。
- `ToolRouter`：本地工具注册和调用。
- `MCPToolRouter`：外部 MCP 工具路由。
- `hooks`：工具调用前后的扩展点。
- `system_prompt`：动态或静态 prompt。

这个结构的关键是：工具不直接拿完整 `Agent`，而是只拿 `ToolContext`。

## Agent Loop

`Agent::agent_loop()` 是完整版本的主循环：

```text
micro compact
  -> 如果上下文超限，自动 compact
  -> 构造模型请求
  -> 合并本地工具和 MCP 工具 schema
  -> 调用模型
  -> 处理 prompt too long / transient error / max tokens
  -> 如果没有 tool_use，结束本轮
  -> 执行 tool_use
  -> 回填 tool_result
  -> 如果调用 compact 工具，手动 compact
  -> 继续循环
```

这仍然是 s01 的闭环，只是每个阶段都增加了真实 agent 需要的工程边界。

## ToolRouter

工具系统延续 s20 的结构，核心在 [`src/tool/mod.rs`](./src/tool/mod.rs)：

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;

    async fn call(&self, context: ToolContext, input: Value) -> Result<String>;
}
```

`ToolRouter` 保存工具名到工具实现的映射：

```rust
pub struct ToolRouter {
    tools: HashMap<String, Box<dyn Tool>>,
}
```

本地工具通过链式注册：

```rust
ToolRouter::new()
    .route(BashTool)
    .route(ReadFileTool)
    .route(TaskCreateTool)
    .route(WorktreeRunTool)
```

每个工具使用强类型输入，并通过 `schemars` 生成模型可见的 `input_schema`。这样 schema 和 Rust 输入类型保持同源。

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

它只包含工具执行需要的业务依赖，不包含：

- LLM client
- 对话上下文
- 权限策略
- recovery 状态
- hooks

这些属于 agent runtime。这个边界能避免工具层反向控制整个 agent。

## 本地工具

`toolset()` 注册完整工具集：

- 基础工具：`add`、`bash`、`read_file`、`write_file`、`edit_file`
- skill：`load_skill`
- memory：`save_memory`
- compact：`compact`
- subagent：`task`
- task：`task_create`、`task_get`、`task_list`、`task_update`
- background：`background_run`、`background_check`
- cron：`cron_create`、`cron_delete`、`cron_list`
- team：`spawn_teammate`、`list_teammates`、`send_message`、`broadcast`、`read_inbox`、`plan_approval`、`shutdown_request`、`shutdown_response`
- worktree：`worktree_create`、`worktree_list`、`worktree_status`、`worktree_run`、`worktree_events`

subagent 使用单独的 `subagent_toolset()`，只开放：

- `bash`
- `read_file`
- `write_file`
- `edit_file`

这样子代理可以独立探索和修改文件，但不会递归创建新的 team、cron、background 或 worktree 控制面。

## Store

完整版本新增了一个重要抽象：[`src/store.rs`](./src/store.rs)。

Store 层只处理持久化文件读写，不表达业务规则：

- `StoreRoot`：表示 `.claude` 状态根目录，并限制路径不能逃出 root。
- `Store<T>`：表示一个 typed JSON 文件，也支持 JSONL append/read_all。
- `CollectionStore<T>`：表示一组 typed JSON 文件。

Domain manager 持有 store，并暴露业务方法。

例如 task：

```rust
pub struct TaskManager {
    tasks: CollectionStore<TaskRecord>,
    index: Store<TaskIndex>,
}
```

外部只调用：

```rust
task_manager.create(...)
task_manager.update(...)
task_manager.list(...)
```

调用方不需要知道 task 文件如何命名，也不应该直接操作 `CollectionStore<TaskRecord>`。

## 状态目录

默认状态根目录是当前工作区的 `.claude`：

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
    <task>.json
  team/
    config.json
    inbox/
      <owner>.json
  worktrees/
    index.json
```

这些文件是 agent harness 的 durable state。它们不依赖当前模型上下文，因此可以支持跨轮次恢复和查询。

## Domain Manager

完整版本把有明确业务语义的状态都收进 manager。

`TaskManager`：

- 创建任务。
- 查询任务。
- 更新状态、owner 和依赖。
- 完成任务时清理依赖。

`BackgroundManager`：

- 启动后台命令。
- 保存后台任务状态。
- 查询任务输出和退出状态。

`CronScheduler`：

- 创建定时任务。
- 删除定时任务。
- 列出计划。

`TeammateManager`：

- 保存 teammate 配置。
- 发送消息和广播。
- 读取 inbox。
- 发送 plan approval / shutdown 这类 protocol request。

`WorktreeManager`：

- 创建 git worktree。
- 列出 worktree。
- 查看状态。
- 在 worktree 内执行命令。
- 记录 worktree events。

Store 负责“怎么落盘”，manager 负责“这件业务应该怎么变化”。

## Permission

权限系统在 [`src/permission.rs`](./src/permission.rs)，执行工具前统一检查。

模式包括：

- `Default`：读操作允许，写操作和高危操作询问。
- `Plan`：只读操作允许，写操作拒绝。
- `Auto`：读操作和非高危写操作允许，高危操作询问。

权限判断发生在 tool dispatch 之前：

```text
tool_use
  -> PermissionManager::check
  -> allow / ask / deny
  -> ToolRouter 或 MCPToolRouter
```

因此 `PermissionManager` 属于 `AgentRuntime`，不是 `ToolContext`。

## Hook

hook 定义在 [`src/hook.rs`](./src/hook.rs)，当前保留三类：

- `SessionStart`
- `PreToolUse`
- `PostToolUse`

主循环已经接入 `PreToolUse` 和 `PostToolUse`：

```text
PreToolUse
  -> permission check
  -> execute tool
  -> PostToolUse
  -> tool_result
```

`PreToolUse` 可以修改工具输入或阻断调用。`PostToolUse` 可以修改工具输出或阻断结果返回。

## Compact 和 Recovery

完整版本同时处理上下文压缩和错误恢复。

compact 机制包括：

- 每轮前执行 `micro_compact`。
- 上下文估算超过 `CONTEXT_LIMIT` 时自动 compact。
- `compact` 工具触发手动 compact。
- compact 前写入 transcript。
- 大的 `bash` 输出会落盘并返回预览。
- 最近读取文件会被记录到 compact prompt 中。

recovery 机制包括：

- prompt too long：触发 compact 后重试。
- transient transport error：指数退避后重试。
- max tokens：注入 continuation message 继续生成。

这些逻辑让长任务不至于因为上下文过长或临时网络问题直接中断。

## System Prompt

动态 system prompt 由 [`src/prompt.rs`](./src/prompt.rs) 和 [`src/system_prompt_template.md`](./src/system_prompt_template.md) 生成。

它包含：

- agent 角色和工作目录。
- 行为约束。
- 可用技能摘要。
- memory 内容。
- `CLAUDE.md` 指令。
- 当前日期、工作目录、模型、平台等动态上下文。
- memory 使用指引。

主 agent 每次 loop 会构造动态 prompt。subagent 使用静态 prompt，避免继承主 agent 的完整上下文。

## Skill 和 Memory

skill 系统扫描：

```text
skills/*/SKILL.md
```

启动时只把技能摘要放进 system prompt，需要时通过 `load_skill` 加载全文。

memory 系统使用 `.claude/memory`，通过 `save_memory` 写入偏好、事实、反馈和引用。system prompt 会加载 memory summary，让 agent 跨轮次保留重要信息。

## MCP

MCP 接入在 [`src/mcp.rs`](./src/mcp.rs)。

启动时扫描：

```text
.claude-plugin/plugin.json
```

manifest 中的 `mcpServers` 会被启动并连接。每个外部工具会转换成模型可见的 tool spec，名称格式是：

```text
mcp__<plugin>__<server>__<tool>
```

执行时：

- 普通工具走 `ToolRouter`。
- `mcp__` 前缀工具走 `MCPToolRouter`。

两类工具都会先经过权限判断，再把结果作为 `tool_result` 回填到上下文。

## Worktree 和 Subagent

`task` 工具可以启动 fresh-context subagent。subagent 拥有自己的 `Agent` 实例和独立上下文，但共享 `ToolContext` 中的基础依赖。

worktree 工具让 agent 可以把任务放进独立 git worktree：

```text
worktree_create
  -> task tool 启动 subagent
  -> worktree_run / worktree_status
  -> worktree_events
```

当前 `sfull` 的 worktree 实现是整合版的最小能力，重点是让完整 runtime 中具备隔离执行入口，而不是实现完整分支合并流程。

## 和前面章节的关系

`sfull` 可以看作一条完整 Rust agent harness 路线的落点：

- s01-s04：agent loop、工具、计划、subagent。
- s05-s08：skill、compact、permission、hook。
- s09-s14：memory、prompt、recovery、task、background、cron。
- s15-s18：team、protocol、autonomous worker、worktree isolation。
- s19-s20：MCP plugin 和 tool router refactor。

在 `sfull` 中，这些能力不再分散在独立 crate，而是通过统一的 runtime、router、context 和 store 组织起来。

## 本章的局限

- `ToolContext` 仍然是单一类型，root agent、subagent、teammate 没有各自独立 context。
- team protocol 是最小消息协议，尚未实现完整 autonomous teammate runtime。
- worktree 只覆盖创建、状态、运行和事件，不负责 merge / rebase / conflict。
- Store 没有跨进程文件锁。
- MCP 只覆盖 stdio server 和 tool call，没有 resources、prompts、OAuth 和自动重连。
- hooks 有类型和注册方法，但还没有完整配置文件驱动。

## 推荐阅读顺序

1. [`src/main.rs`](./src/main.rs)：理解初始化顺序。
2. [`src/lib.rs`](./src/lib.rs)：理解完整 agent loop。
3. [`src/tool/mod.rs`](./src/tool/mod.rs)：理解 ToolRouter 和 ToolContext。
4. [`src/store.rs`](./src/store.rs)：理解 StoreRoot / Store / CollectionStore。
5. [`src/permission.rs`](./src/permission.rs)、[`src/compact.rs`](./src/compact.rs)、[`src/recovery.rs`](./src/recovery.rs)：理解运行时控制。
6. [`src/task.rs`](./src/task.rs)、[`src/team.rs`](./src/team.rs)、[`src/worktree.rs`](./src/worktree.rs)：理解 domain manager。
7. [`src/mcp.rs`](./src/mcp.rs)：理解外部 MCP 工具如何进入同一条工具链路。

## 验证

检查完整版本：

```bash
cargo check -p sfull
```

运行测试：

```bash
cargo test -p sfull
```

检查整个 workspace：

```bash
cargo check --workspace
```
