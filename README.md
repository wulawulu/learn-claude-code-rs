# AI Agent 开发项目

这是一个逐步构建AI Agent系统的Rust项目，分为三个阶段（s1-s3），每个阶段都在前一阶段的基础上增加功能和改进架构。

## 项目概述

本项目展示了如何从零开始构建一个支持工具调用的AI Agent系统，逐步演进架构设计。项目使用Rust语言和Anthropic AI SDK，通过DeepSeek模型实现智能代理。

## 项目结构

```
.
├── Cargo.toml          # 工作区配置文件
├── s01_agent_loop/     # 第一阶段：基础Agent循环
├── s02_tool_use/       # 第二阶段：工具系统重构
└── s03_todo_write/     # 第三阶段：任务规划系统
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

## 技术栈

- **编程语言**：Rust 2024 edition
- **AI SDK**：anthropic-ai-sdk 0.2.27
- **异步运行时**：tokio 1.51.0
- **错误处理**：anyhow 1.0.102
- **序列化**：serde 1.0.228, serde_json 1.0.149
- **环境变量**：dotenvy 0.15.7

## 运行要求

1. 设置环境变量：
   ```bash
   export ANTHROPIC_API_KEY="your_api_key"
   export ANTHROPIC_BASE_URL="https://api.deepseek.com"
   ```

2. 运行任意阶段：
   ```bash
   cd s01_agent_loop  # 或 s02_tool_use / s03_todo_write
   cargo run
   ```

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

## 设计模式演进

1. **s1**：简单循环模式
2. **s2**：策略模式（工具抽象）
3. **s3**：观察者模式/钩子模式（生命周期管理）

## 未来扩展方向

基于当前架构，可以进一步扩展：
1. **插件系统**：支持动态加载工具和组件
2. **工作流引擎**：定义复杂的任务执行流程
3. **记忆系统**：长期记忆和上下文管理
4. **多Agent协作**：多个Agent协同工作
5. **可视化界面**：任务执行状态的可视化展示

## 学习价值

本项目展示了AI Agent系统的渐进式开发过程：
- 从简单到复杂的架构演进
- 设计模式在实际项目中的应用
- Rust trait系统在构建可扩展架构中的优势
- AI Agent生命周期管理的核心概念

通过这三个阶段的演进，可以清晰地看到如何从一个简单的AI对话系统逐步构建成一个支持复杂任务规划和状态管理的智能Agent系统。
