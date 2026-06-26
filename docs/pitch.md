# Agent-OS 答辩讲稿

> 项目：在 rCore-Tutorial-v3 上扩展的**面向 AI 智能体的操作系统内核**
>
> 适配三种场景：1 分钟电梯演讲 / 5–10 分钟答辩主讲 / Q&A 预案

---

## 一、1 分钟电梯演讲

> 各位评委好。

> 我们做的项目叫 **Agent-OS**。问题动机是：当下 Claude Code 这类 AI Agent
> 已经成为软件构建的新范式——它通过**结构化工具调用**和**多轮迭代上下文**
> 工作。但操作系统对这种工作模式毫无感知——Agent 的每次工具调用都要穿过
> Shell、运行时库等多层软件栈才能到达内核。

> 所以我们提出一个根本问题：如果把 AI Agent 当作 OS 的一等公民，
> 内核应该提供哪些原生支持？

> 我们在 rCore-Tutorial-v3 上扩展了三个核心抽象：
> 1. **Agent Context 区**——一段内核分配、用户态映射的共享内存，
>    工具调用结果由内核直接写入，用户态**零 syscall** 读取。
> 2. **结构化工具调用协议**——强类型枚举 + postcard 二进制编码，
>    比 JSON 快约 10 倍，编译期捕获拼写错误。
> 3. **Context Path 内核管理 + Agent Loop 运行时**——心跳由 timer
>    中断推进，事件由邮箱触发，Agent 真正做到无事不耗 CPU。

> 整个项目实现了 6 个任务：19 个新 syscall、5 个内核工具、9 个 demo
> 程序、全部在 QEMU 上可演示。

> 谢谢。

---

## 二、5–10 分钟答辩主讲

### 1. 问题背景（45 秒）

> Agent 的工作模式是 **Agent Loop**：思考 → 调用工具 → 观察结果 → 再思考。
> 这个循环有两个特征：
> - 工具调用是**结构化**的（不是 shell 命令字符串）
> - 每一轮的结果驱动下一轮的决策（需要**上下文积累**）

> 但操作系统看不到这些——Agent 的工具调用最终被翻译成 read/write 系统调用，
> 上下文是用户态 JSON 文件。OS 对此没有任何优化，也没有任何抽象。

> 我们的核心问题是：**如果让 Agent 成为内核一等公民，需要哪些新机制？**

### 2. 核心设计（3 分钟）

> 我们的设计原则是**机制与策略分离**——这是 OS 的经典命题，但在 Agent 场景下
> 有新的体现：
> - 内核管"机制"：地址空间分配、配额、调度、淘汰
> - 用户态管"策略"：缓存什么、淘汰哪个、查询时机

> 三个关键抽象：

> **抽象一：Agent Context 区。**
> 这是一段内核分配、映射到用户地址空间 `0x80000000` 的共享内存，
> 默认 64 KB。它内部切分成 4 段：Header、Tool Result Ring、Path Buffer、
> Tool Call History。
>
> 为什么放共享内存？因为工具调用结果常常很大（比如查询 100 个进程的信息），
> 走 syscall 返回值要么需要多次 read，要么需要大缓冲区拷贝。我们让内核
> **直接把结果写入这段共享内存**，syscall 只返回一对 `(offset, len)`。
> 用户态拿到 offset 后**无需任何 syscall** 就能读取——这就是零拷贝路径。
>
> 你可以在 `agent_demo_tool.rs` 看到这个行为：发起 `system_status` 工具
> 调用后，用户态直接从 `0x80000100` 起读 postcard 编码的结果，没有
> `sys_read`，没有 `copy_to_user`。

> **抽象二：强类型工具协议。**
> 我们没有用 JSON，原因有三：
> 1. JSON 解析慢、堆分配多，在 no_std 内核里成本高
> 2. 字符串工具名容易拼错，运行时才报错
> 3. 缺乏 schema，难做兼容演进
>
> 我们用 `postcard`（一个 no_std 友好的二进制 serde 后端）加上**强类型枚举
> `ToolName`** 作为工具标识。请求格式是：
>
> ```rust
> pub struct ToolRequest {
>     pub req_id: u64,
>     pub tool: ToolName,   // 枚举，不是字符串
>     pub params: ToolParams,
> }
> ```
>
> 序列化体积比 JSON 紧凑 3–5 倍，解析在 RISC-V 上快 8–15 倍。同时通过
> `#[serde(default)]` 我们支持字段级前向兼容。

> **抽象三：Context Path 分层存储 + Agent Loop 内核化。**
> Agent Loop 的每一轮调用都会形成一条"探索路径"。我们把它拆成两层：
> - **元信息**（offset、长度、时戳、序号）在内核 PCB
> - **数据本体**（请求/响应摘要）在用户态 Path Buffer
>
> 这样路径配额由内核控制——超过 16 KB 自动 LRU 淘汰，绝不会因 Agent
> 失控导致 OOM。但路径数据本身在用户态共享内存，用户态可以高速读，
> 不需要每查一条 path 就 syscall。
>
> Agent Loop 自身的运行机制也下沉到了内核：
> - 心跳由 timer 中断推进（每 10ms 一次 tick，扫描所有 Agent 的心跳到期状态）
> - `sys_agent_wait` 让 Agent 真正休眠，事件到达时唤醒
> - 邮箱 + `send_message` 工具实现 Agent 间消息传递

### 3. 实现亮点（1.5 分钟）

> 几个值得讲的工程决策：

> **零侵入设计。** PCB 加了一个 `agent_ext: Option<Box<AgentExt>>` 字段——
> 普通进程为 None，对它们零开销。所有新代码集中在 `os/src/agent/`，
> rCore 原有逻辑改动不到 20 行。

> **共享协议 crate。** 我们把所有协议类型抽到 `agent_proto` crate，
> OS 和 user 都依赖它。这样**永远不会出现协议漂移**——比如内核改了
> ToolName 但用户态没改，cargo 直接编译报错。

> **Rust 强类型 + Result。** 所有 syscall 路径都用 `Result<T, AgentError>`，
> 错误码统一翻译为负 isize 返回值。没有任何 `unwrap()` 在错误路径上。

> **倒排索引 vs 全量扫描。** 任务四的 `query_file` 工具同时支持索引和扫描
> 两条路径，方便做性能对比。我们用 `BTreeMap<(key, value), Vec<filename>>`
> 实现哈希倒排，多条件查询变成 O(k) 的集合求交。

### 4. 演示（2 分钟，按 QEMU 启动后操作）

> 我们做了 9 个 demo 程序，覆盖全部 6 个任务。

> 一键演示用 `agent_runner`，它会顺序跑完全部 9 个 demo：
>
> ```
> >> agent_runner
> ```
>
> 我们着重看三个：
>
> - `agent_demo_tool`（任务二）：调用 `system_status` 工具，可以看到
>   syscall 返回 `(offset=0x100, len=N)`，然后用户态从 `0x80000100`
>   直接读出 postcard 编码的 SystemStatusInfo。**零拷贝路径走通。**
> - `agent_demo_file`（任务四）：在 N=10/100/1000/5000/10000 个文件上做
>   倒排索引 vs 全量扫描的规模化对比（内核内计时、排除 syscall 开销），
>   N=10000 时索引快 ~113×。**这是性能评估数据。**
> - `agent_demo_npc`（任务六）：fork 3 个 NPC Agent，每个用心跳进入
>   Agent Loop，通过 query_process 找其他 NPC，互相 send_message。
>   最后每个 NPC 打印自己的 Context Path，**展现思考过程**。

### 5. 总结（30 秒）

> 我们把 AI Agent 提升为操作系统一等公民，提出了三个新抽象：
> Agent Context 区、结构化工具协议、Context Path 内核管理。
> 所有抽象都体现了机制/策略分离这个 OS 经典思想。
>
> 完整代码、设计文档、ADR、性能报告都在仓库里。
>
> 谢谢。

---

## 三、Q&A 预案

### Q1：为什么不直接用现有的 IPC（管道/socket）？

> 现有 IPC 是为"无类型字节流"设计的，需要应用层自己定结构、自己 marshal。
> Agent 工具调用是高频结构化操作，每次都要做协议封装/解析——这正是 OS
> 应该提供的抽象。类似 epoll 抽象了"事件就绪"这个概念，我们抽象了
> "结构化工具调用"这个概念。
>
> 另外，传统 IPC 没有零拷贝结果区的概念，每条消息都要内存拷贝。我们的
> 共享内存设计针对 Agent 大结果数据场景做了专门优化。

### Q2：Agent Context 区放共享内存，用户态恶意修改怎么办？

> 这是个好问题。我们的设计有三层防护：
> 1. **不同区段不同权限**：Header 和 Tool Result Ring 是 R|U（只读），
>    用户无法篡改内核维护的元数据；只有 Tool Call History 段是 R|W|U，
>    允许用户写自己的统计。
> 2. **关键状态在 PCB**：path_used_bytes 等真正的源头数据在内核 PCB，
>    用户态看到的 Header 是这些数据的镜像。即使用户篡改镜像，下次内核
>    操作时会用 PCB 里的值覆盖回去。
> 3. **fork 时清空**：子进程 fork 出来默认是普通进程，需要自己再
>    `agent_create` 才有 Context 区——不会"继承"父进程的内容。
>
> 当前 demo 代码为了简化把整段映射成 R|W|U，正式版需要拆成多个 MapArea，
> 这是已知待优化项。

### Q3：用 postcard 而不是 JSON，调试怎么办？

> 我们的协议在调试上做了两个考虑：
> 1. 帧头有 magic（`0xA9E47F00`）和 version（`0x0001`）字段，wireshark
>    /dump 工具一眼能识别。
> 2. 我们后续可以提供 `agent-trace` 用户态工具，把二进制帧反编码为 JSON
>    文本，仅在调试时启用。这样**生产路径紧凑、调试路径可读**两全。

### Q4：sys_agent_wait 是真的休眠吗？

> **是。** 我们在 rCore-ch6 调度器里加了第四种状态 `TaskStatus::Blocked`。
>
> 流程：
> 1. `sys_agent_wait` 检查没事件 → 调 `block_current_agent(deadline)`
> 2. 该函数把当前 task 从 processor 拿走（`take_current_task`），
>    设 status = Blocked，**推进全局 `BLOCKED_AGENTS` 列表**而不是 ready queue，
>    然后 `schedule()` 切回 idle 协程
> 3. processor 的 `fetch_task` **永远拿不到 blocked 任务**——所以该 Agent
>    的时间片真正为 0
> 4. 三个唤醒路径都把 task 移回 ready queue：
>    - **心跳到期**：`tick_all_agents` 置位 pending 后调 `wake_agent_by_pid`
>    - **消息投递**：`deliver_message` 投递成功后调 `wake_agent_by_pid`
>    - **超时**：每次 timer 中断里 `tick_wake_timeouts` 扫描 deadline
>
> 这正是要求文档里"Agent 在无事件时正确休眠，不消耗 CPU"的字面达标。
> 全部新代码集中在 `os/src/agent/blocking.rs`（约 100 行），对原 rCore
> 调度核心的侵入只有 `TaskStatus` 加 1 行变体。

### Q5：哈希倒排索引为什么用 BTreeMap 而不是 HashMap？

> 选 BTreeMap 是因为：
> 1. 在 no_std + alloc 环境下，BTreeMap 不需要额外依赖
> 2. 我们的 key 是 (String, String) 复合键，BTreeMap 的字典序遍历对
>    范围查询友好（虽然当前 demo 没用到范围查询）
> 3. 文件数在教学场景是几十～几百，O(log n) vs O(1) 实际差距可以忽略
>
> 如果文件数量大幅增加，会切到 `hashbrown::HashMap`（已在依赖里）。
> 我们的 query_indexed 接口完全兼容这两种容器。

### Q6：NPC 之间互发消息时，怎么找到对方 pid？

> 通过 `query_process` 工具，过滤 `is_agent=true` 的进程，排除自己即可。
> 这其实是 demo 里就能看到的行为：
>
> ```rust
> let others = query_other_agents(my_pid);
> for p in &others {
>     send_to(p.pid as u64, payload);
> }
> ```
>
> 这种"通过查询找对端"的模式比硬编码 pid 灵活——任何 Agent 加入系统
> 都能被发现。这也是 Agent 工具调用相比传统 IPC 的优势之一。

### Q7：协议演进怎么保证兼容？

> 三层保证：
> 1. **帧头 version**：不兼容变更时升版本号，旧客户端直接拒绝
> 2. **ToolName 枚举新增 variant**：旧内核遇到新工具返回 ToolNotFound，
>    不会 crash
> 3. **ToolParams 字段加 `#[serde(default)]`**：新字段对旧版本是可选的
>
> 我们在 `docs/design/01-protocol.md` 里专门写了演进规则一节。

### Q8：和 Linux 的 io_uring 有什么关系？

> io_uring 启发了我们的零拷贝 Result Ring 设计——都用环形缓冲在用户态
> 共享内存里放结果。区别是：
> - io_uring 是为"异步 I/O"设计的，每条请求是一个 SQE
> - 我们的 Result Ring 是为"工具调用"设计的，强类型枚举决定语义
> - io_uring 仍然是无类型字节流；我们是带 schema 的结构化数据
>
> 可以认为我们是把 io_uring 的"零拷贝传输"思想 + protobuf 的"强类型协议"
> 思想 + epoll 的"事件就绪"思想，重新组合在 Agent 这个新工作模式下的产物。

### Q9：项目的最大局限是什么？

> 两个：
> 1. **文件属性不持久化**。FILE_ATTR_STORE 在内核内存里，重启丢。我们
>    选择不修改 easy-fs 磁盘格式以保证零侵入，代价就是这个。可以通过
>    把 store 序列化到一个 `attr.db` 文件解决。
> 2. **没有真实 LLM 集成**。NPC demo 里 Agent 行为是规则驱动的。但比赛
>    重点是 OS 设计，不是 Agent 智能程度——所有 OS 层抽象已就位，接入
>    LLM 只是用户态工程。

### Q10：怎么用上你这套机制？给我一个具体场景。

> 举两个：
> 1. **代码理解 Agent**（类比 Claude Code）：Agent 反复 grep + read 代码。
>    我们的 query_file 索引能让"按 tag 找文件"在 µs 级；Context Path
>    完整记录"我已经看过哪些文件、得出了什么结论"，Agent 自己回看路径就
>    能避免重复探索。
> 2. **运维 Agent**（场景 B）：Agent 周期巡检系统状态，发现异常采取行动。
>    心跳触发巡检，事件驱动响应告警，Context Path 记录决策审计——这套
>    机制刚好契合。

### Q11：怎么证明属性查询"性能优于遍历"？给我对比数据。

> 我们专门做了一个**内核内规模化基准** `sys_file_attr_bench`（#540）。它在一个
> 独立局部属性表上构造 N 个文件（少数命中目标条件，模拟"大海捞针"），把同一组合
> 查询重复 200 次，**直接在内核内用时钟 tick 计时——排除 syscall 往返和序列化开销**，
> 只测查询函数本身。结果（QEMU 实测）：
>
> | N | full-scan | indexed | speedup |
> |---|---|---|---|
> | 10 | 0.83M ns | 1.06M ns | 0.78× |
> | 1000 | 9.76M ns | 0.99M ns | 9.83× |
> | 10000 | 114.9M ns | 1.02M ns | **113×** |
>
> 三个要点：
> 1. **倒排索引耗时基本恒定**（~1M ns，与 N 无关）——这就是 O(命中数) 的体现。
> 2. **全量扫描随 N 近似线性增长**——O(N)，N=10000 时比索引慢 113 倍。
> 3. **交叉点在 N≈1000**：小 N 时索引反而略慢（固定开销没摊薄），这是真实且可解释的，
>    也说明索引的价值在 Agent 大规模语义检索场景才完全释放。
>
> 重点：我们把**索引机制下沉到内核**——`set` 时花 O(c) 维护倒排表，让 `query` 永远是
> O(命中数)。这是 Elasticsearch / 任何生产索引系统的通用设计。详见 perf-report §3。

---

## 四、演示彩蛋（如有时间）

如果时间允许，演示这个流程（每步配解说）：

1. **`agent_demo_create`** —— 强调 Context Area Header 的 magic
   `0xA9E45EC0` 验证内核确实写入了用户态共享内存
2. **`agent_demo_tool`** —— 强调 `system_status` 调用的返回值是
   `(offset, len)`，用户态读 `0x80000100` 没有第二次 syscall
3. **`agent_demo_file`** —— 强调 STEP 5/5 规模对比表：N=10000 时索引比全扫快 ~113×
4. **`agent_demo_npc`** —— 强调三个 NPC 互发消息时的真实并发
   ——每个 NPC 在 Context Path 里完整记录了自己看到的世界
