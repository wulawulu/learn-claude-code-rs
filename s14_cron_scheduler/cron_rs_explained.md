# `cron.rs` 代码讲解

这份文档专门解释 `s14_cron_scheduler/src/cron.rs`。

目标不是重复 `s14.md` 的章节说明，而是帮助你第二天回来时快速重新建立代码心智模型：

- 这个文件到底负责什么
- 每个结构体和方法各自做什么
- 为什么这样拆
- 读代码时应该按什么顺序看

---

## 先给一个总览

`cron.rs` 主要负责四件事：

1. 定义“调度任务长什么样”
2. 管理这些任务的内存状态和磁盘持久化
3. 在后台按时间检查哪些任务应该触发
4. 把触发结果放进通知队列，等主循环来取

所以它本质上是：

- **调度状态层**
- **后台检查层**
- **通知桥接层**

它并不直接调模型，也不直接修改对话上下文。
真正把通知塞回 conversation 的逻辑在 `lib.rs` 的 `agent_loop()`。

---

## 建议阅读顺序

如果你明天回来重新看代码，建议按这个顺序：

1. `ScheduleMode` / `PersistenceMode`
2. `ScheduledTaskRecord`
3. `CronNotification` / `MissedTask`
4. `CronScheduler` / `SharedCronScheduler`
5. `start()` / `stop()`
6. `create()` / `delete()` / `list_tasks()`
7. `check_loop()`
8. `check_tasks()`
9. `load_durable()` / `save_durable()`
10. 末尾的辅助函数：`parse_schedule()`、`schedule_matches()`、`compute_jitter()`

也就是说：

- 先看“数据”
- 再看“控制流”
- 最后看“工具函数”

---

## 第一部分：模式类型

### `ScheduleMode`

```rust
pub enum ScheduleMode {
    Recurring,
    OneShot,
}
```

它表示一个调度任务是：

- `Recurring`：会重复触发
- `OneShot`：只触发一次，触发后移除

这里不用 `bool recurring`，而是用 enum，是为了让代码更可读。

比如：

- `matches!(task.mode, ScheduleMode::Recurring)`

会比：

- `if task.recurring { ... }`

更清楚，因为它直接表达了领域概念，而不是靠布尔值猜语义。

### `PersistenceMode`

```rust
pub enum PersistenceMode {
    Session,
    Durable,
}
```

它表示任务是：

- `Session`：只存在当前进程内存里
- `Durable`：要写到 `.claude/scheduled_tasks.json`

同样，这里没有继续用 `bool durable`，而是提升成 enum。

这两个 enum 都派生了 `Display`，所以打印时可以直接得到比较自然的字符串。

---

## 第二部分：调度任务记录

### `ScheduledTaskRecord`

```rust
pub struct ScheduledTaskRecord {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub mode: ScheduleMode,
    pub persistence: PersistenceMode,
    pub created_at: i64,
    pub last_fired_at: Option<i64>,
    pub jitter_offset_minutes: u64,
}
```

这是这个文件最核心的数据结构。

每个字段含义如下：

- `id`
  - 调度任务的唯一标识
- `cron`
  - cron 表达式原文
- `prompt`
  - 任务命中时要注入回主循环的消息内容
- `mode`
  - recurring 还是 one-shot
- `persistence`
  - session 还是 durable
- `created_at`
  - 任务创建时间
- `last_fired_at`
  - 上次触发时间，用于 missed task 检测
- `jitter_offset_minutes`
  - 如果需要 jitter，就记录一个偏移分钟数

这个 struct 的作用是：

- 它既是内存中的任务表示
- 也是磁盘上的持久格式

也就是说，`.claude/scheduled_tasks.json` 序列化和反序列化的主要载体就是它。

### 它为什么要实现 `Display`

`Display` 主要是给：

- `cron_list`
- `/cron`

这种“列出任务”场景服务。

所以这个实现本质上是“人类可读摘要视图”，而不是完整 JSON 视图。

---

## 第三部分：通知与 missed task

### `CronNotification`

```rust
pub struct CronNotification {
    pub task_id: String,
    pub prompt: String,
}
```

这个类型很小，但很关键。

它表示：

- 某个调度任务已经命中
- 现在应该把这个 prompt 重新交还给主循环

它不是持久任务记录，也不是完整任务对象，而是：

- 一次“已经触发”的短消息

所以它只保留：

- `task_id`
- `prompt`

### `MissedTask`

```rust
pub struct MissedTask {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub missed_at: String,
}
```

它用于“会话关闭期间本来应该触发，但实际上错过了”的场景。

当前代码里已经有 `detect_missed_tasks()`，但这部分还没有进一步接回 UI 交互。

也就是说：

- 现在能力已经有了
- 只是主流程还没用到它

这很合理，因为这一章重点还是 scheduler 本身。

---

## 第四部分：为什么有 `CronScheduler` 和 `SharedCronScheduler`

### `CronScheduler`

```rust
struct CronScheduler {
    claude_dir: PathBuf,
    tasks: Mutex<HashMap<String, ScheduledTaskRecord>>,
    notification_tx: mpsc::Sender<CronNotification>,
    notification_rx: Mutex<mpsc::Receiver<CronNotification>>,
    stop_requested: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
    next_id: AtomicU64,
}
```

这是内部状态结构。

可以把它理解成：

- “真正存数据的那一层”

里面每一项都服务于 scheduler 自己：

- `claude_dir`
  - durable 文件和锁文件都在这里
- `tasks`
  - 当前已知的调度任务表
- `notification_tx` / `notification_rx`
  - 后台检查和主循环之间的队列
- `stop_requested`
  - 后台 loop 的停止标志
- `worker`
  - 后台检查任务的句柄
- `next_id`
  - 用于生成新的 task id

### `SharedCronScheduler`

```rust
pub struct SharedCronScheduler {
    inner: Arc<CronScheduler>,
}
```

它是对外暴露的共享句柄。

它的作用很简单：

- 让 scheduler 可以在多个地方安全共享
- 比如：
  - `main.rs`
  - `LoopState`
  - cron 工具模块

之所以不把所有方法都直接挂在裸 `Arc<CronScheduler>` 上，是为了：

- 把共享性包装起来
- 对外暴露更整洁的 API

所以你可以把它理解成：

- `CronScheduler` = 内部实现
- `SharedCronScheduler` = 对外门面

---

## 第五部分：启动和停止

### `start()`

这是 scheduler 的启动入口。

它做三件事：

1. `load_durable()`
   - 把磁盘上的 durable tasks 读进来
2. `stop_requested = false`
   - 确保后台 loop 可以正常运行
3. 如果后台 loop 还没启动，就 `tokio::spawn(check_loop())`

这里最容易卡住的是：

```rust
let mut worker = self.inner.worker.lock()?;
```

这不是“拿到后台 worker 本身”，而是：

- 锁住 scheduler 里存放 `JoinHandle` 的那个位置

然后：

```rust
if worker.is_some() {
    return self.task_count();
}
```

表示：

- 如果后台检查 loop 已经启动过了
- 就不要再重复启动第二个

最后：

```rust
*worker = Some(tokio::spawn(async move {
    scheduler.check_loop().await;
}));
```

表示：

- 启动后台任务
- 并把返回的 `JoinHandle` 存起来

### `stop()`

`stop()` 做两件事：

1. 设置 `stop_requested = true`
2. 取出 `JoinHandle` 并 `await`

这里最重要的理解是：

- `stop_requested = true` 只是发停止信号
- `handle.await` 才是等它真正停完

所以 `.await` 本身不会“让它停”，而是：

- 等后台 loop 读到停止标志并退出之后
- 再让 `stop()` 返回

---

## 第六部分：增删查调度任务

### `create()`

它主要负责：

1. 校验 cron 表达式
2. 生成 task id
3. 计算 mode / persistence
4. 对 recurring 任务计算 jitter
5. 写入内存 store
6. 如果是 durable，就立刻保存到磁盘

返回值是：

- 一条方便给模型或用户看的确认字符串

### `delete()`

它会：

1. 从内存里移除任务
2. 如果删掉的是 durable task，就更新磁盘文件

### `list_tasks()`

它只是把当前任务表转成可读字符串。

所以这三个方法就是：

- scheduler 对外最基本的 CRUD 接口

---

## 第七部分：通知队列

### `drain_notifications()`

```rust
pub fn drain_notifications(&self) -> Vec<CronNotification>
```

它的作用是：

- 把当前所有已经触发、但还没被消费的 notification 一次性拿出来

这个方法本身不改上下文，也不调模型。

它只是做：

- queue -> `Vec<CronNotification>`

然后由 `LoopState::inject_scheduled_notifications()` 再把这些内容转成 `user` 消息。

所以这是一层很明确的边界：

- `cron.rs` 只负责通知生产和提供通知
- `lib.rs` 负责真正注入对话

---

## 第八部分：后台检查循环

### `check_loop()`

这是整个文件里最重要的控制流。

它的职责是：

- 周期性醒来
- 决定当前进程有没有资格做 cron 检查
- 如果有，就检查任务是否命中

它的流程大致是：

1. 创建 `interval`
2. 每次 `tick().await`
3. 检查 `stop_requested`
4. 如果当前还没拿到 cron 文件锁，就尝试拿
5. 如果拿到锁，就检查当前分钟是否需要执行一轮任务匹配
6. 命中的任务交给 `check_tasks()`

这里有两个关键点。

### 1. 为什么要锁文件

因为多个进程可能同时跑在同一个工作目录下。

如果没有锁：

- 每个进程都会认为自己应该触发任务
- 同一任务可能会被 enqueue 多次

所以当前实现通过 `.claude/cron.lock` 做了“leader 选举”：

- 拿到锁的那个进程负责调度检查
- 没拿到锁的就只是空转等待

### 2. 为什么还要 `last_check_minute`

因为 `interval` 是按秒 tick 的。

如果没有这层：

- 在同一分钟内会重复检查很多次
- 同一 cron 任务可能被反复触发

所以这里做了一个最小控制：

- 同一分钟只真正执行一次检查

---

## 第九部分：任务匹配与清理

### `check_tasks()`

这是后台调度逻辑的核心。

它会遍历当前所有任务，并做几件事：

1. 检查 recurring 任务是否超龄，需要 auto-expire
2. 解析任务的 cron 表达式
3. 应用 jitter 后计算 `check_time`
4. 判断该任务此刻是否命中
5. 如果命中：
   - 生成 `CronNotification`
   - 更新 `last_fired_at`
   - one-shot 的话标记为待删除
6. 最后统一：
   - 发送通知
   - 删除 expired / one-shot
   - 如有必要则保存 durable 快照

这里的重点是：

- 匹配、通知、清理是同一轮逻辑里完成的

这样可以保证：

- 任务命中后状态会一起更新
- 不会只触发通知却忘记更新持久状态

---

## 第十部分：durable 加载与保存

### `load_durable()`

它负责从：

- `.claude/scheduled_tasks.json`

加载 durable tasks。

这里有一段容易误读：

```rust
self.with_tasks(|store| {
    store.retain(|_, task| !task.persistence.is_durable());
    for task in tasks {
        store.insert(task.id.clone(), task);
    }
    Ok(())
})?;
```

这段的意思不是“保留 durable”。

它的真实语义是：

- 先删掉当前内存里的旧 durable tasks
- 保留 session-only tasks
- 再把磁盘里的 durable tasks 装回来

这样做是为了：

- 刷新 durable 部分
- 不误删当前内存中的 session-only tasks

### `save_durable()` / `save_durable_snapshot()`

这两个方法负责把 durable tasks 写回磁盘。

分成两层是为了：

- 一层负责从当前 store 里筛 durable
- 一层负责真正写文件

这样 `check_tasks()` 里如果已经有 durable 快照，就可以直接调用 `save_durable_snapshot()`，少做一次重复收集。

---

## 第十一部分：cron 解析与匹配

### `parse_schedule()`

它直接使用 `cron` crate：

- 把 cron 字符串解析成 `Schedule`

这里没有再额外实现一层适配或手写解析器。

也就是说：

- 这里选择的是 `cron` crate 的原生表达式语义

### `schedule_matches()`

它的职责是：

- 给定一个 `Schedule`
- 判断某个具体分钟点是否命中

之所以不是直接“问 schedule 现在匹不匹配”，是因为 `cron` crate 的接口更偏向：

- 给出一个时间点
- 计算后续触发序列

所以当前实现的思路是：

1. 把当前时间截断到整分钟
2. 取“前一分钟”
3. 询问 `schedule.after(previous).next()`
4. 如果下一个触发点正好是当前分钟，就认为命中

这其实是在把 crate 的“生成未来时间点”接口，转成我们想要的“这一分钟是否命中”语义。

### `compute_jitter()`

它的逻辑比较简单：

- 如果 cron 的分钟字段正好是 `0` 或 `30`
- 就基于表达式 hash 算一个稳定的小偏移

这个设计的目的很明确：

- 避免大量 recurring 任务都堆在整点/半点同时触发

这里的 jitter 不是随机抖动，而是：

- 对同一表达式稳定可重复

这样更容易测试，也更容易理解。

---

## 第十二部分：为什么这些 helper 不再继续拆出去

看到这里你可能会问：

- 为什么 `parse_schedule()`、`compute_jitter()`、`schedule_matches()` 这些函数还留在 `cron.rs` 里？

原因是当前这一章里，它们仍然都强耦合于 scheduler 领域。

它们不是通用工具函数，而是：

- scheduler 用来解释 cron 记录
- scheduler 用来决定是否触发
- scheduler 用来分散触发负载

所以现在把它们留在 `cron.rs` 底部，反而是高内聚的。

只有当以后出现：

- 另一个模块也要独立复用 cron 匹配能力

才值得单独拆出去。

---

## 最后给你一个一句话总结

如果明天你只想先快速回忆这个文件在干嘛，可以记这一句：

> `cron.rs` 负责保存未来任务、在后台按时间检查它们、把命中的任务变成通知队列，再交给主循环重新注入对话。
