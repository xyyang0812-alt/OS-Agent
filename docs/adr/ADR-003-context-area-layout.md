# ADR-003: Agent Context 区设计

**状态**：已采纳
**日期**：2026-06-22

## 上下文

任务一与任务三的核心：Agent Context 区放在用户态还是内核态？放在用户态的话，如何放置、如何与内核维护的元信息同步？

赛题文档明确给出方向：**机制（在内核） + 策略（在用户态）**。

## 决策

Agent Context 区是一段**由内核分配、映射到用户空间、用户只读、内核可写**的共享内存。

- 虚拟基址：`0x8000_0000`
- 默认大小：64 KB（16 页），通过 `AgentCreateCfg.context_area_size` 可配
- 切分为 4 段：Header / Tool Result Ring / Path Buffer / Tool Call History

## 理由

1. **零拷贝**：工具调用结果直接由内核写入共享内存，用户态 `read by offset` 完全无 syscall。这是协议设计的杠杆点。
2. **安全**：用户态对内核维护的关键元信息（如 Path Buffer 的 head/tail）只读，杜绝越权篡改。
3. **配额可控**：内核完全掌握 Context 区大小、淘汰策略，满足任务三验收要求（"不导致内核 OOM"）。
4. **零开销 for 普通进程**：`agent_ext: Option<Box<AgentExt>>`，普通进程为 `None`，不引入额外内存。

## 区段切分依据

| 区段 | 大小 | 容量预算 | 用途 |
|---|---|---|---|
| Header | 256 B | 单个固定结构 | 版本号、各段偏移、写入序号（防撕裂） |
| Tool Result Ring | 16 KB | ≥64 次调用结果 | 工具调用返回数据 |
| Path Buffer | 32 KB | ≥256 个 path node | 任务三的核心数据 |
| Tool Call History | 8 KB | 统计信息、Trace | 用户态可写（仅此区） |
| Reserved | ~8 KB | 预留扩展 | — |

## 撕裂防护方案

内核每次写入 Header 之外的区段前后，会原子地递增 Header 的 `seq_number`：

```text
write_seq = header.seq_number.fetch_add(1, ...);  // 奇数：写入中
... 写数据 ...
header.seq_number.store(write_seq + 1, ...);      // 偶数：完成
```

用户态读取时，若 `seq_number` 为奇数 → 重试；若两次读取间 `seq_number` 改变 → 重试。简化的 seqlock 模式。

## 后果

- 用户态库必须使用 `volatile` 读取 Header
- 性能比纯 syscall 提升约 5-10 倍（详见性能报告章节）

## 风险

- 用户态恶意进程虽不能写，但能 fork 后偷看 Context 区——子进程继承时**应清空**该区域（已在 fork 路径里处理）
- 多线程 Agent 时需要用户态自加锁——目前 ch6 单线程，未来若升级 ch8 需要重新设计
