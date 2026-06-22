# ADR-001: 基线内核选择 —— rCore-Tutorial-v3 (ch6)

**状态**：已采纳
**日期**：2026-06-22

## 上下文

赛题要求基于教学操作系统（uCore / rCore）扩展。可选基线：

| 候选 | 特征 |
|---|---|
| uCore (C) | 经典 C 语言、调试工具链成熟、社区资料多 |
| rCore-Tutorial-v3 ch6 | Rust、带 easy-fs 文件系统 |
| rCore-Tutorial-v3 ch8 | Rust、带线程/协程/信号量、调度更灵活 |

## 决策

选择 **rCore-Tutorial-v3 ch6**。

## 理由

1. **任务四需要文件系统**。ch6 自带 easy-fs，省去从零构建的工作量；ch8 也含文件系统但叠加了线程/协程模型，复杂度过高。
2. **Rust 类型系统是创新性加分项**。强类型工具协议、所有权管理 Context 区生命周期、`Result<T, AgentError>`，都是答辩亮点。
3. **rCore 模块边界清晰**。`task/`、`mm/`、`fs/`、`syscall/` 各成一族，便于插入新子系统而不污染原代码。
4. **代码安全**。比赛评分有"无内存泄漏、死锁等内核级缺陷"硬性要求，Rust 编译期 + `UPSafeCell` 大大降低出错概率。

## 后果

- 团队需要熟悉 Rust（学习成本，但对应人员现有掌握）
- 调试需要用 GDB + QEMU 而非 printf
- 第三方 crate 必须 `no_std` 兼容（已验证 `postcard`、`hashbrown`、`spin` 均可用）

## 备选回退

如果中途发现 ch6 的 easy-fs 难以扩展属性，可降级为"在 inode 旁额外维护属性表文件"——不修改 easy-fs 本身，而是在 `fs/attr.rs` 层做装饰。
