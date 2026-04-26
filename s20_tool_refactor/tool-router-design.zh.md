# Tool Router 设计记录

本文整理这次关于 tool 重构的讨论：用类似 Axum 的 router、强类型输入、自动生成
`input_schema`、显式注入 tool 上下文，并把 agent 运行时状态和 tool 可访问依赖分开。

本文档描述的是 `s20_tool_refactor` 当前采用的实现。早期讨论里曾考虑过更接近 Axum
的 `Context<C>` / `Json<T>` extractor 和泛型 `ToolRouter<C>`，但当前阶段选择了更简单的
版本：`Agent` 拥有 `ToolContext`，`ToolRouter` 只负责动态分发，宏生成的 tool wrapper
是零状态 struct。

## 目标

- 减少每个 tool 的重复模板代码。
- 让业务 tool 写成普通 async 函数。
- 用强类型输入 struct 替代手动访问 `serde_json::Value`。
- 从 Rust 输入 struct 自动生成 Anthropic tool 的 `input_schema`。
- 通过显式 `ToolContext` 参数给 tool handler 注入共享依赖。
- 区分 agent runtime state 和 tool-accessible context。

## 当前问题

现在每个 tool 通常都要写 struct、构造函数、`Tool` trait 实现、手动参数解析和手动
schema：

```rust
pub struct LoadSkillTool {
    registry: Arc<SkillRegistry>,
}

pub fn load_skill_tool(registry: Arc<SkillRegistry>) -> Box<dyn Tool> {
    Box::new(LoadSkillTool { registry }) as Box<dyn Tool>
}

#[async_trait]
impl Tool for LoadSkillTool {
    async fn invoke(&mut self, input: &serde_json::Value) -> anyhow::Result<String> {
        let name = input
            .get("name")
            .and_then(|value| value.as_str())
            .context("Invalid name")?;

        Ok(self.registry.load_full_text(name))
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: "load_skill".to_string(),
            description: Some("Load a skill.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "required": ["name"]
            }),
        }
    }
}
```

这段代码混合了四件事：

- tool 元数据。
- 动态分发。
- JSON 输入解析。
- 业务逻辑。

重构目标是把元数据、解析、schema 生成和动态分发收进框架代码或宏生成代码里。

## `schemars` 和 `serde` 的分工

`schemars` 用来从 Rust 类型生成 JSON Schema。

`serde` 仍然负责把 JSON 反序列化成 Rust struct。

```rust
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoadSkillInput {
    pub name: String,
}
```

解析 tool 输入：

```rust
let input: LoadSkillInput = serde_json::from_value(value)?;
```

生成 tool input schema：

```rust
let schema = schemars::schema_for!(LoadSkillInput);
let input_schema = serde_json::to_value(schema)?;
```

可选字段天然对应非 required 字段：

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileInput {
    pub path: String,
    pub limit: Option<u64>,
}
```

这里 `path` 是必填，`limit` 是可选。

结论：

```text
serde_json::Value -> struct      使用 serde_json::from_value
struct -> input_schema           使用 schemars
```

## ToolContext

注入给 tool handler 的值不建议叫 `AgentState`。

旧的 `LoopState` 实际混合了很多概念：

- LLM client。
- 对话上下文。
- tool registry。
- permission policy。
- recovery / compact 状态。
- skill、memory、task、worktree manager 等业务依赖。

新设计里，注入给 tool handler 的值只表示 tool 可访问的业务依赖。`ToolContext` 这个名字
更准确。

示例：

```rust
#[derive(Clone)]
pub struct ToolContext {
    pub skill_registry: Arc<SkillRegistry>,
    pub memory_manager: Arc<Mutex<MemoryManager>>,
    pub task_manager: SharedTaskManager,
    pub worktree_manager: SharedWorktreeManager,
    pub work_dir: PathBuf,
}
```

当前实现没有引入 extractor。handler 直接接收 `ToolContext` 和强类型 input：

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoadSkillInput {
    pub name: String,
}

pub async fn load_skill(
    ctx: ToolContext,
    input: LoadSkillInput,
) -> anyhow::Result<String> {
    Ok(ctx.skill_registry.load_full_text(&input.name))
}
```

这种写法少一层概念，适合当前教程阶段。后续如果 handler 参数类型变多，再引入
`Context<C>`、`Json<T>` 这类 extractor 也不晚。

## Agent 结构

更合理的结构是：agent 拥有 runtime state，也拥有 tool context。router 不应该拥有
context。

```rust
pub struct Agent {
    runtime: AgentRuntime,
    tool_context: ToolContext,
    tools: ToolRouter,
}

pub struct AgentRuntime {
    client: AnthropicClient,
    context: Vec<Message>,
    system_prompt: String,
    permission_manager: Option<PermissionManager>,
    max_round: usize,
}

pub struct ToolRouter {
    tools: HashMap<String, Box<dyn Tool>>,
}
```

tool 执行链路：

```text
Agent::agent_loop()
  -> model 返回 tool_use
  -> permission_manager 检查 tool name 和原始 JSON input
  -> tools.call(&tool_context, name, input)
  -> ToolRouter clone 一份便宜的 ToolContext
  -> 宏生成的 tool wrapper 解析强类型 input
  -> 执行业务 handler
```

这样不会形成循环依赖：

```text
Agent 拥有 ToolContext
Agent 拥有 ToolRouter
ToolRouter 拥有 tool wrappers
Tool wrapper 不拥有 Agent
ToolContext 不拥有 ToolRouter
```

## PermissionManager 应该放在哪里

以 `s07_permission_system` 为例，`PermissionManager` 应该留在 agent runtime，而不是放进
`ToolContext`。

原因是权限检查发生在 tool 执行前：

```rust
let decision = self.permission_manager.check(name, input);
```

这是 agent loop 的执行策略，不是某个 tool handler 应该随意访问的业务依赖。

所以它属于 runtime：

```rust
pub struct AgentRuntime {
    permission_manager: Option<PermissionManager>,
}
```

而这些属于 tool context：

```rust
pub struct ToolContext {
    skill_registry: Arc<SkillRegistry>,
    task_manager: SharedTaskManager,
    work_dir: PathBuf,
}
```

## Tool Trait 仍然需要，但只作为内部接口

业务代码不应该再手写 `impl Tool`。

但框架内部仍然需要一个 object-safe trait 做动态分发，因为模型返回的只有 tool name 和
JSON input。

```rust
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> serde_json::Value;

    async fn call(
        &self,
        context: ToolContext,
        input: serde_json::Value,
    ) -> anyhow::Result<String>;
}
```

router 存储这些 trait object：

```rust
pub struct ToolRouter {
    tools: HashMap<String, Box<dyn Tool>>,
}
```

但是每个具体 tool 的 `Tool` 实现应该由宏生成，而不是业务代码手写。

## 宏方案

当前支持两类业务写法。

需要访问共享依赖的 tool 显式接收 `ToolContext` 和 input struct：

```rust
#[lccr::tool(
    name = "load_skill",
    description = "Load the full body of a named skill into the current context."
)]
pub async fn load_skill(
    ctx: ToolContext,
    input: LoadSkillInput,
) -> anyhow::Result<String> {
    Ok(ctx.skill_registry.load_full_text(&input.name))
}
```

纯函数 tool 可以直接把函数参数作为 JSON input 字段：

```rust
#[tool(name = "add", description = "Add two integers.")]
pub async fn add(
    #[schemars(description = "Left integer operand.")] a: i64,
    #[schemars(description = "Right integer operand.")] b: i64,
) -> i64 {
    a + b
}
```

宏可以生成类似下面的 wrapper：

```rust
pub struct LoadSkillTool;

#[async_trait::async_trait]
impl Tool for LoadSkillTool {
    fn name(&self) -> &'static str {
        "load_skill"
    }

    fn description(&self) -> &'static str {
        "Load the full body of a named skill into the current context."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(LoadSkillInput))
            .expect("schema generation should not fail")
    }

    async fn call(
        &self,
        context: ToolContext,
        input: serde_json::Value,
    ) -> anyhow::Result<String> {
        let input: LoadSkillInput = serde_json::from_value(input)?;
        load_skill(context, input).await
    }
}
```

关键点是：每个注册后的 handler 运行时仍然对应一个内部 struct，但这个 struct 由宏生成。
业务代码只保留 tool 逻辑。这个 struct 是零状态的，不保存 `Arc<ToolContext>` 或 context
副本；context 由 `Agent` 持有，并在调用时传给 router。

## Router API

适合宏生成 wrapper 的 router API 可以是：

```rust
let router = ToolRouter::new()
    .route(LoadSkillTool)
    .route(ReadFileTool)
    .route(EditFileTool);
```

router 形状：

```rust
impl ToolRouter {
    pub fn route<T>(mut self, tool: T) -> Self
    where
        T: Tool + 'static,
    {
        self.tools.insert(tool.name().to_string(), Box::new(tool));
        self
    }

    pub async fn call(
        &self,
        context: &ToolContext,
        name: &str,
        input: serde_json::Value,
    ) -> anyhow::Result<String> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {name}"))?;

        tool.call(context.clone(), input).await
    }
}
```

`ToolContext` 应该设计成 clone 很便宜，一般内部放 `Arc`：

```rust
#[derive(Clone)]
pub struct ToolContext {
    pub skill_registry: Arc<SkillRegistry>,
    pub work_dir: PathBuf,
}
```

因此这里的 `context.clone()` 只是浅 clone。`Arc` 字段增加引用计数，`PathBuf` 拷贝路径
值，不会让每个 tool wrapper 长期持有一份共享状态。

## Root Agent、Subagent 和 Teammate

每个 agent 实例都应该有匹配的 runtime、tool context 和 router。

当前 `s20` 只有一个具体的 `ToolContext`。后续如果 root agent、subagent、teammate 的
能力边界差异变大，可以再把 `Tool` / `ToolRouter` 泛型化，让不同 agent 类型使用不同
context 类型和不同 tools。

root agent：

```rust
pub struct RootToolContext {
    pub skill_registry: Arc<SkillRegistry>,
    pub memory_manager: Arc<Mutex<MemoryManager>>,
    pub task_manager: SharedTaskManager,
    pub worktree_manager: SharedWorktreeManager,
    pub work_dir: PathBuf,
}
```

subagent：

```rust
pub struct SubAgentToolContext {
    pub task_manager: SharedTaskManager,
    pub work_dir: PathBuf,
}
```

teammate：

```rust
pub struct TeammateToolContext {
    pub teammate_name: String,
    pub teammate_manager: SharedTeammateManager,
    pub task_manager: SharedTaskManager,
    pub work_dir: PathBuf,
}
```

这样能力边界更明确。如果 subagent 不应该访问 memory，就不要把 `memory_manager` 放进
`SubAgentToolContext`。

## 后续演进方向

当前阶段已经完成：

1. 用 `schemars` 从 input 类型生成 `input_schema`。
2. 定义内部 `Tool` trait 和只保存工具表的 `ToolRouter`。
3. 重构 agent loop，让 `Agent` 同时拥有 `tool_context: ToolContext` 和 `tools: ToolRouter`。
4. 添加 `#[tool(...)]` proc macro，用宏生成零状态 wrapper。
5. 把常见 tool 迁移成函数 handler。

可以留到后续的增强：

1. 引入 `Context<C>` / `Json<T>` extractor，让 handler 参数更接近 Axum 风格。
2. 把 `Tool` / `ToolRouter` 泛型化成 `Tool<C>` / `ToolRouter<C>`。
3. 为 root agent、subagent、teammate 定义不同的 tool context 类型。
4. 增加更多 extractor，例如权限信息、tool call id、取消信号等。

## 关键结论

- `schemars` 负责生成 `input_schema`。
- `serde_json::from_value` 负责把 `Value` 解析成输入 struct。
- `ToolContext` 是注入给 tool handler 的业务依赖，不是完整 agent state。
- `PermissionManager` 属于 agent runtime，不属于 tool context。
- `ToolRouter` 不应该拥有 context；`Agent` 拥有 context，调用 tool 时传给 router。
- 宏生成的 wrapper 是零状态 struct，不保存 context。
- 内部仍然需要 `Tool` trait 做动态分发。
- 业务 tool 应该写成 async 函数。
- 倾向用宏生成 wrapper，而不是用 blanket handler implementation。
