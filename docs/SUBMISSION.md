# Agent-OS 提交清单

> 评委一眼能看到的完整交付物索引

## 一、可执行交付物

### Kernel

| 产物 | 路径 | 体积 |
|---|---|---|
| Kernel ELF（带调试符号）| `os/target/riscv64gc-unknown-none-elf/release/os` | 6.6 MB |
| Kernel binary（strip 后）| 通过 `make build` 生成 | **148 KB** |

Kernel 在 RISC-V 64 + QEMU virt + RustSBI 0.3.1 上启动。

### User 程序（9 个 Agent demo + 1 个 NPC 工人 + 1 个 runner）

| 程序 | 任务 | 功能 |
|---|---|---|
| `agent_demo_create` | 1 | Agent 进程创建 + Context 区映射（含 `loop_state` 输出）|
| `agent_demo_coexist` | 1 | **普通进程 + Agent 进程并存验证**（任务一第 3 验收点）|
| `agent_demo_tool` | 2 | 5 个工具调用 + send_message↔mailbox 往返 |
| `agent_demo_path` | 3 | Context Path push/query/rollback/clear + LRU |
| `agent_demo_file` | 4 | 属性设置/查询/删除 + **规模化性能对比**（N=10000 索引快 ~113×）|
| `agent_demo_loop` | 5 | 心跳 + 事件驱动 + mailbox + **真休眠** |
| `agent_demo_fileevent` | 5b | 文件变更事件（`EVENT_FILE_MODIFIED`）驱动唤醒 |
| `agent_demo_priority` | 5c | 优先级调度（多 Agent 协调）|
| `agent_demo_npc` | 6 | NPC 综合演示（fork 3 个 NPC）|
| `agent_npc_worker` | 6 | NPC worker（被 npc demo 启动）|
| `agent_runner` | — | **一键** runner，顺序跑完全部 9 个 demo |

Rust + postcard + 严格无 `unwrap` 路径。

## 二、源码统计

```
新增代码总量：3341 行（含 5% 注释 + 设计文档引用）
```

| 模块 | 行数级别 | 内容 |
|---|---|---|
| `agent_proto/src/lib.rs` | 240 | 共享协议（OS + user 唯一真相源）|
| `os/src/agent/mod.rs` | 22 | 子系统入口 |
| `os/src/agent/error.rs` | 50 | `AgentError` + 错误码翻译 |
| `os/src/agent/protocol.rs` | 8 | re-export `agent_proto` |
| `os/src/agent/pcb_ext.rs` | 105 | PCB 扩展字段 `AgentExt` |
| `os/src/agent/context_area.rs` | 175 | Agent Context 区分配 + Header + 跨地址空间读写 |
| `os/src/agent/context_path.rs` | 215 | Path Buffer 写入 + FIFO/LRU 淘汰 |
| `os/src/agent/event_bus.rs` | — | 心跳 tick + 邮箱投递 + 文件事件广播 |
| `os/src/agent/file_attr.rs` | — | 属性表 + 倒排索引 + 规模化性能基准（旁路 easy-fs）|
| `os/src/agent/tool/registry.rs` | — | ToolDispatcher |
| `os/src/agent/tool/handlers.rs` | — | 5 个内核工具实现 |
| `os/src/syscall/agent.rs` | — | 19 个 syscall 入口 |
| `user/src/lib.rs` | — | 用户态 Agent API |
| `user/src/syscall.rs` | — | 19 个 syscall 包装 |
| `user/src/bin/agent_*` | 9 个文件 | demo 程序 + NPC 工人 + runner |

对原 rCore 代码的改动集中在少数几处：
- `os/src/main.rs`：注册 `mod agent`，调 `init_demo_attrs()`
- `os/src/task/task.rs`：加 `agent_ext: Option<Box<AgentExt>>` + `priority` 字段
- `os/src/task/manager.rs`：就绪队列 `fetch` 改为按优先级取
- `os/src/syscall/mod.rs`：注册 19 个新 syscall 编号
- `os/src/trap/mod.rs`：syscall 扩 6 参，timer 中断调 `tick_all_agents`

## 三、文档（12 份）

### 设计文档（3 份）

| 文档 | 路径 | 内容要点 |
|---|---|---|
| 总览 | `docs/design/00-overview.md` | 架构 mermaid + 设计原则 |
| 协议规格 | `docs/design/01-protocol.md` | 帧格式、postcard、演进规则 |
| Syscall 规格 | `docs/design/02-syscall-spec.md` | 19 个 syscall 精确定义 |

### 架构决策记录（3 份 ADR）

| ADR | 内容 |
|---|---|
| ADR-001 | 基线选 rCore-ch6 的理由 |
| ADR-002 | 协议格式选 postcard 而非 JSON 的论证 |
| ADR-003 | Context 区设计 + seqlock 撕裂防护 |

### 用户 / 答辩 / 评估文档（6 份）

| 文档 | 用途 |
|---|---|
| `README.md` | 项目门面，含 3 张 mermaid 图 + 验收/评分对照 |
| `docs/QUICKSTART.md` | 从 0 到跑通 5 分钟指南 |
| `docs/pitch.md` | **答辩讲稿**：1min / 5-10min / Q&A 预案 |
| `docs/perf-report.md` | 性能报告（含 113× 规模化实测） |
| `docs/impl-report-tool-call.md` | 零拷贝工具调用编码报告 |
| `docs/SUBMISSION.md` | 本文档：交付物清单 |
| `docs/run-log.md` | **实测运行记录**：真实 QEMU 输出 + 分析 |

## 四、syscall 一览（19 个新增）

| 编号 | 名称 | 任务 | 已实现 |
|---|---|---|---|
| 500 | `sys_agent_create` | 1 | ✅ |
| 501 | `sys_agent_info` | 1 | ✅ |
| 510 | `sys_tool_call` | 2 | ✅ |
| 511 | `sys_tool_list` | 2 | ✅ |
| 520 | `sys_context_push` | 3 | ✅ |
| 521 | `sys_context_query` | 3 | ✅ |
| 522 | `sys_context_rollback` | 3 | ✅ |
| 523 | `sys_context_clear` | 3 | ✅ |
| 530 | `sys_agent_heartbeat_set` | 5 | ✅ |
| 531 | `sys_agent_heartbeat_stop` | 5 | ✅ |
| 532 | `sys_agent_watch` | 5 | ✅ |
| 533 | `sys_agent_wait` | 5 | ✅ |
| 534 | `sys_agent_unwatch` | 5 | ✅ |
| 535 | `sys_mailbox_recv` | 5 | ✅ |
| 536 | `sys_agent_set_loop_state` | 1+5 | ✅ |
| 537 | `sys_file_attr_del` | 4 | ✅ |
| 538 | `sys_file_attr_set` | 4 | ✅ |
| 539 | `sys_agent_set_priority` | 5 | ✅ |
| 540 | `sys_file_attr_bench` | 4 | ✅ |

## 五、内核工具一览（5 个）

| 工具 | 输入参数 | 输出 | 任务 |
|---|---|---|---|
| `system_status` | 无 | `SystemStatusInfo` | 2 |
| `query_process` | `status`, `ty` 过滤 | `QueryResult<ProcessInfo>` | 2 |
| `read_context` | `target_type`, `target_id` | `ProcessInfo` | 2 |
| `send_message` | `target_pid`, `payload` | 状态码 | 2 + 5 |
| `query_file` | `tag`, `owner`, `keyword`, `use_index` | `QueryResult<FileInfo>` | 4 |

## 六、评分维度自评

### 创新性（30%）— 自评 9/10

- ✅ 把 Agent 提升为 OS 一等公民的完整抽象（Context 区/协议/Path/Loop）
- ✅ 零拷贝 Context 区（io_uring 思想下沉到工具调用）
- ✅ 强类型二进制协议（ToolName 枚举 + postcard）
- ✅ 倒排索引在 OS 教学项目里的实践
- ✅ 机制/策略分离贯穿全程
- ✅ **真休眠**：扩展 `TaskStatus::Blocked` + BLOCKED_AGENTS 列表，
  Agent 无事件时时间片为 0（任务五验收"不消耗 CPU"字面达标）

### 完整性（20%）— 自评 10/10

- ✅ 6 个任务全部实现（基础 3 + 进阶 2 + 综合 1）
- ✅ 进阶任务四：实现 1+2+3+4 四项（要求"至少 2 项"），含查询性能优于遍历的实测对比（~113×）
- ✅ 进阶任务五：实现 1+2+3+4 四项（要求"至少 2 项"），含可选优先级调度机制
- ✅ 综合任务六：整合 1+2+3+5 四个模块（要求"至少 3 个"）

### 代码质量（25%）— 自评 9/10

- ✅ Zero `unwrap()` on error paths
- ✅ Strong type errors（`Result<T, AgentError>`）
- ✅ Cargo 编译 0 warning（OS + user 都过）
- ✅ 子系统内聚（全在 `os/src/agent/`）
- ✅ 共享 crate 消除协议漂移风险
- ✅ 普通进程零开销（`Option<Box<...>>`）
- 已知 TODO（未阻塞）：Context 区段权限拆分 / FILE_ATTR_STORE 持久化

### 文档完整性（25%）— 自评 10/10

- ✅ README + 3 张 mermaid 图（架构/地址空间/Tool Call 时序）+ 验收/评分逐条对照
- ✅ 3 份设计文档 + 3 份 ADR
- ✅ 答辩讲稿（3 个时长版本 + Q&A 预案）
- ✅ 性能报告（含 113× 规模化实测）+ 零拷贝工具调用编码报告
- ✅ QUICKSTART 用户指南 + 端到端运行记录
- ✅ 本提交清单

## 七、快速运行（评委复现 3 分钟）

```bash
# 1) 环境（首次）
sudo apt install -y qemu-system-misc
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
. "$HOME/.cargo/env"
cargo install cargo-binutils
rustup component add llvm-tools-preview rust-src

# 2) 编译并启动
cd agent-os/os
make run

# 3) 在 QEMU 的 shell 里
>> agent_runner

# 4) 预期：
# =============================================
#   SUMMARY: 9 PASS, 0 FAIL (out of 9)
#   AGENT-OS ALL DEMOS PASS
# =============================================
```
