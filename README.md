# 渐进式Agent Harness教学项目

这是一个逐步构建AI Agent系统的Rust教学项目，从基础的Agent循环开始，逐步增加功能，最终构建一个完整的、支持权限管理的Agent系统。项目分为7个阶段（s1-s7），每个阶段都在前一阶段的基础上增加新功能和改进架构。

## 项目概述

本项目展示了如何从零开始构建一个功能完整的AI Agent系统，逐步演进架构设计。项目使用Rust语言和Anthropic AI SDK，通过DeepSeek模型实现智能代理，涵盖了工具调用、任务规划、子代理、技能加载、上下文压缩和权限管理等核心概念。

## 项目结构

```
.
├── Cargo.toml              # 工作区配置文件
├── s01_agent_loop/         # 第一阶段：基础Agent循环
├── s02_tool_use/           # 第二阶段：模块化工具系统
├── s03_todo_write/         # 第三阶段：任务规划系统
├── s04_subagent/           # 第四阶段：子代理系统
├── s05_skill_loading/      # 第五阶段：技能加载系统
├── s06_context_compact/    # 第六阶段：上下文压缩
├── s07_permission_system/  # 第七阶段：权限管理系统
└── skills/                 # 技能目录
    ├── agent-builder/      # Agent构建技能
    ├── code-review/        # 代码审查技能
    ├── mcp-builder/        # MCP构建技能
    └── pdf/                # PDF处理技能
```

## 第一阶段：s01_agent_loop

### 功能特点
- **基础Agent循环**：实现了最基本的AI Agent交互循环
- **单一工具支持**：仅支持`bash`命令执行工具
- **简单状态管理**：使用`LoopState`结构管理对话上下文
- **安全限制**：包含危险命令黑名单和超时保护

### 核心实现
- 使用Anthropic AI SDK与DeepSeek模型通信
- 实现`run_bash`函数执行shell命令
- 基本的工具调用和执行流程
- 简单的对话上下文管理

### 关键技术
- 异步命令执行带超时保护
- 危险命令黑名单检查
- 基本的消息提取和格式化

### 局限性
- 工具系统硬编码，难以扩展
- 状态管理简单，缺乏灵活性
- 没有任务规划能力

## 第二阶段：s02_tool_use

### 功能改进
- **模块化工具系统**：将工具抽象为`Tool` trait，支持动态注册
- **多工具支持**：新增`read_file`、`write_file`、`edit_file`工具
- **消息规范化**：实现`normalize_messages`函数处理对话历史
- **工具执行优化**：统一的工具调用接口

### 架构改进
- 引入`Tool` trait定义工具接口：
  ```rust
  pub trait Tool: Send + Sync {
      fn name(&self) -> &str;
      fn description(&self) -> &str;
      fn tool_spec(&self) -> anthropic_ai_sdk::types::message::Tool;
      async fn invoke(&self, input: &serde_json::Value) -> anyhow::Result<String>;
  }
  ```
- 使用`HashMap`管理工具集合，支持动态添加
- 分离工具定义和执行逻辑

### 新增工具
1. **bash**：执行shell命令
2. **read_file**：读取文件内容
3. **write_file**：写入文件内容
4. **edit_file**：替换文件中的文本

### 消息规范化
- 合并连续相同角色的消息
- 处理孤立的tool_use块
- 确保对话历史的完整性

## 第三阶段：s03_todo_write

### 功能增强
- **任务规划系统**：新增`todo`工具支持多步骤任务管理
- **智能提醒机制**：自动检测长时间未更新计划并提醒
- **状态感知**：Agent能够跟踪任务执行进度

### 架构演进
- **生命周期管理**：引入状态感知的Agent组件
- **Hook机制**：工具不仅在被调用时执行，还能参与Agent生命周期
- **智能状态跟踪**：`LoopState`跟踪`todo_rounds_since_update`

### 核心特性
1. **任务规划工具**：
   - 支持创建、更新任务计划
   - 跟踪任务执行状态（pending/in_progress/completed）
   - 确保多步骤任务只有一个步骤处于`in_progress`状态

2. **智能提醒系统**：
   - 每3轮未更新计划时自动提醒
   - 使用`<reminder>`标签提示刷新计划
   - 工具调用后重置提醒计数器

3. **改进的System Prompt**：
   ```rust
   const SYSTEM: &str = r#"You are a coding agent.
   Use the todo tool for multi-step work.
   Keep exactly one step in_progress when a task has multiple steps.
   Refresh the plan as work advances. Prefer tools over prose.
   "#;
   ```

### 架构洞察
第三阶段的关键认知升级是从"工具调用"到"生命周期管理"的转变：
- **Tool**：被调用的能力（被动执行）
- **Hook/Component**：系统行为的参与者（主动参与）
- **生命周期管理**：Agent在不同阶段触发不同组件

## 第四阶段：s04_subagent

### 功能增强
- **子代理系统**：新增`task`工具支持创建子代理执行特定任务
- **任务委派**：主Agent可以将复杂任务委派给子代理
- **结果整合**：子代理执行结果自动整合到主对话中

### 架构改进
- **子代理工具**：实现`sub_agent_tool`创建独立的Agent实例
- **任务隔离**：子代理在独立的上下文中执行任务
- **结果传递**：子代理执行结果作为工具输出返回

### 核心特性
1. **任务委派**：
   - 主Agent可以创建子代理处理特定任务
   - 子代理拥有独立的对话上下文
   - 支持任务描述和特定指令

2. **系统提示优化**：
   ```rust
   let system = format!(
       "You are a coding agent at {}. Use the task tool to delegate exploration or subtasks.",
       std::env::current_dir()?.display()
   );
   ```

3. **工具复用**：子代理复用主Agent的工具集

### 应用场景
- 探索性任务（如文件系统探索）
- 独立的功能模块开发
- 需要隔离上下文的复杂任务

## 第五阶段：s05_skill_loading

### 功能增强
- **技能加载系统**：新增`load_skill`工具动态加载专业技能
- **技能注册表**：实现`SkillRegistry`管理可用技能
- **动态系统提示**：根据加载的技能动态更新系统提示

### 架构改进
- **技能目录结构**：`skills/`目录包含各种专业技能
- **技能描述**：每个技能包含名称、描述和系统提示
- **技能缓存**：使用`Arc`共享技能注册表

### 核心特性
1. **技能加载工具**：
   - 动态加载技能文件
   - 将技能内容作为系统提示注入
   - 支持技能描述查询

2. **技能注册表**：
   - 扫描技能目录
   - 缓存技能信息
   - 提供技能描述列表

3. **动态系统提示**：
   ```rust
   let system = format!(
       r#"You are a coding agent at {}.
   Use load_skill when a task needs specialized instructions before you act.

   Skills available:
       {}
   "#,
       std::env::current_dir()?.display(),
       state.skill_registry.describe_available()
   );
   ```

### 预置技能
- **agent-builder**：Agent构建技能
- **code-review**：代码审查技能
- **mcp-builder**：MCP构建技能
- **pdf**：PDF处理技能

## 第六阶段：s06_context_compact

### 功能增强
- **上下文压缩**：新增`compact`工具压缩过长的对话历史
- **自动压缩**：当上下文超过限制时自动触发压缩
- **微压缩**：实现`micro_compact`优化消息格式

### 架构改进
- **上下文大小估计**：估算对话历史的token数量
- **智能压缩策略**：保留关键信息，压缩冗余内容
- **压缩工具**：Agent可以主动请求压缩上下文

### 核心特性
1. **上下文管理**：
   - 估计上下文大小（约50000字符限制）
   - 自动检测和触发压缩
   - 保留最近的关键对话

2. **压缩算法**：
   - 合并连续的系统消息
   - 移除过时的工具调用结果
   - 保留最近的用户查询和Agent响应

3. **系统提示优化**：
   ```rust
   let system = format!(
       r#"You are a coding agent at {}.
   Keep working step by step, and use compact if the conversation gets too long.
   "#,
       std::env::current_dir()?.display(),
   );
   ```

### 技术实现
- **上下文大小估计**：基于字符数的简单估算
- **微压缩**：轻量级的消息格式优化
- **自动触发**：超过阈值时自动执行压缩

## 第七阶段：s07_permission_system

### 功能增强
- **权限管理系统**：实现三种权限模式（Default、Plan、Auto）
- **交互式界面**：使用`inquire`库提供更好的用户交互
- **权限控制**：根据模式控制工具调用的权限

### 架构改进
- **权限管理器**：`PermissionManager`处理权限决策
- **模式选择**：启动时选择权限模式
- **工具包装**：工具调用前检查权限

### 核心特性
1. **三种权限模式**：
   - **Default模式**：每次工具调用都需要用户确认
   - **Plan模式**：执行计划时自动批准，其他需要确认
   - **Auto模式**：自动批准所有工具调用

2. **权限管理器**：
   - 管理当前权限模式
   - 处理权限检查逻辑
   - 提供用户确认界面

3. **交互式界面**：
   - 使用`inquire`库提供美观的CLI界面
   - 模式选择菜单
   - 改进的用户输入提示

### 系统提示
根据权限模式动态调整系统提示，告知Agent当前的权限设置。

## 技术栈

- **编程语言**：Rust 2024 edition
- **AI SDK**：anthropic-ai-sdk 0.2.27
- **异步运行时**：tokio 1.51.0
- **错误处理**：anyhow 1.0.102
- **序列化**：serde 1.0.228, serde_json 1.0.149
- **环境变量**：dotenvy 0.15.7
- **交互式CLI**：inquire 0.10.0（s07）

## 运行要求

1. 设置环境变量：
   ```bash
   export ANTHROPIC_API_KEY="your_api_key"
   export ANTHROPIC_BASE_URL="https://api.deepseek.com"
   ```

2. 运行任意阶段：
   ```bash
   cd s01_agent_loop  # 或 s02_tool_use / s03_todo_write / ...
   cargo run
   ```

3. 对于s07，启动时会提示选择权限模式。

## 项目演进总结

### s1 → s2：从硬编码到模块化
- 工具系统从硬编码变为可扩展的trait系统
- 引入消息规范化处理复杂的对话历史
- 支持多种文件操作工具

### s2 → s3：从工具调用到生命周期管理
- 引入任务规划能力
- 实现状态感知的Agent组件
- 从被动工具调用升级到主动生命周期参与
- 智能提醒机制提升Agent的自主性

### s3 → s4：从单Agent到多Agent协作
- 支持创建子代理处理特定任务
- 实现任务委派和结果整合
- 扩展Agent系统的协作能力

### s4 → s5：从静态到动态技能系统
- 引入技能加载机制
- 支持动态扩展Agent能力
- 实现技能注册表和缓存

### s5 → s6：从无限上下文到智能管理
- 实现上下文压缩机制
- 自动检测和管理上下文大小
- 优化长时间对话的性能

### s6 → s7：从无限制到安全可控
- 引入权限管理系统
- 提供三种权限模式选择
- 增强系统的安全性和可控性

## 设计模式演进

1. **s1**：简单循环模式
2. **s2**：策略模式（工具抽象）
3. **s3**：观察者模式/钩子模式（生命周期管理）
4. **s4**：工厂模式（子代理创建）
5. **s5**：注册表模式（技能管理）
6. **s6**：策略模式（压缩算法）
7. **s7**：状态模式（权限管理）

## 学习价值

本项目展示了AI Agent系统的完整演进过程：
- 从简单到复杂的架构演进
- 设计模式在实际项目中的应用
- Rust trait系统在构建可扩展架构中的优势
- AI Agent核心概念的逐步实现
- 安全性和可控性的重要性

通过这七个阶段的演进，可以清晰地看到如何从一个简单的AI对话系统逐步构建成一个功能完整、安全可控的智能Agent系统，涵盖了现代AI Agent系统的所有核心组件。

## 未来扩展方向

基于当前架构，可以进一步扩展：
1. **持久化存储**：保存对话历史和Agent状态
2. **可视化界面**：Web或GUI界面展示任务执行状态
3. **多模型支持**：集成多个AI模型提供商
4. **工作流引擎**：定义复杂的任务执行流程
5. **插件市场**：社区贡献的技能和工具插件
6. **性能监控**：实时监控Agent性能和资源使用
7. **分布式执行**：支持多个Agent并行执行任务
