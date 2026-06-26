# Agent-OS 快速启动指南

> 从 0 到 "AGENT-OS ALL DEMOS PASS" 只需 5 分钟。

---

## 一、依赖安装（首次运行需要）

```bash
# 1. Rust 工具链（用户态安装，无需 sudo）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none

# 2. 让当前 shell 看到 cargo
. "$HOME/.cargo/env"

# 3. QEMU RISC-V 64（需要 sudo）
sudo apt update
sudo apt install -y qemu-system-misc

# 4. cargo-binutils（生成 kernel.bin 需要）
cargo install cargo-binutils
rustup component add llvm-tools-preview rust-src

# 5. 验证版本
qemu-system-riscv64 --version    # 应 ≥ 7.0
rustc --version
```

---

## 二、编译并启动

```bash
cd /home/xuliyang/tmp/os/agent-os/os
make run
```

`make run` 会自动：
1. 编译 kernel（`os` crate）
2. 编译 user 端所有程序
3. 用 `easy-fs-fuse` 把 user 程序打包进 fs.img
4. 启动 QEMU

首次编译约 1–3 分钟；后续增量编译几秒。

---

## 三、运行 demo

QEMU 启动后会看到 rCore 的 logo，然后进入 shell `>>`。

### 推荐方式：一键跑通

```
>> agent_runner
```

按回车后会顺序跑全部 demo，每个完成后打印 `<<< [n/N] xxx PASS`。
最后看到：

```
=============================================
  SUMMARY: 9 PASS, 0 FAIL (out of 9)
  AGENT-OS ALL DEMOS PASS
=============================================
```

> 注意：`agent_demo_file` 的规模化性能基准（N 最大 10000）在 QEMU 里约需 30–60 秒，
> 跑 `agent_runner` 时这一步会停顿较久，属正常现象，请耐心等待。

### 单 demo 调试

如需单独跑某个任务的 demo：

```
>> agent_demo_create     # 任务一
>> agent_demo_coexist    # 任务一验收（共存）
>> agent_demo_tool       # 任务二
>> agent_demo_path       # 任务三
>> agent_demo_file       # 任务四（含规模化性能对比）
>> agent_demo_loop       # 任务五（心跳 + 邮箱 + 真休眠）
>> agent_demo_fileevent  # 任务五b（文件变更事件唤醒）
>> agent_demo_priority   # 任务五c（优先级调度）
>> agent_demo_npc        # 任务六（NPC 生态综合演示）
```

### 退出 QEMU

```
Ctrl-A  然后  x
```

---

## 四、常见问题

### Q: `Error when executing!` + `Shell: Process N exited with code -4`

**原因**：命令名末尾多了空格或 Tab，shell 把 `"agent_demo_create \0"`
丢给 exec，文件系统找不到带空格的文件。

**解决**：
- 我们已经给 `user_shell` 加了 `line.trim()`，新版会自动剥离首尾空格
- 如果你仍看到这个错误，说明你跑的还是旧版 fs.img——`Ctrl-A x` 退出
  再 `make run` 即可

### Q: `qemu-system-riscv64: command not found`

```bash
sudo apt install -y qemu-system-misc
```

### Q: `linker rust-lld failed: cannot find linker script src/linker.ld`

你直接跑了 `cargo build` 而不是 `make`。请用 `make run` 或 `make build`，
Makefile 会自动复制 `linker-qemu.ld → linker.ld` 再编译。

### Q: `[kernel] IllegalInstruction in application, kernel killed it.`

**原因**：rCore-Tutorial-v3 ch6 默认**不为用户进程开启 FPU**
（`sstatus.FS = Off`）。用户态执行任何 F/D 扩展指令（`fadd.d`、
`fmul.d`、`fdiv.d` 等）都会触发非法指令异常。

**踩坑历史**：早期 `agent_demo_file` 用 `f64` 做"平均耗时"和"倍率"
计算，触发了这个错误。

**解决**：用户态 demo 全部用整数运算。倍率用 `scan_ms * 100 / indexed_ms`
表达成 `X.YZx` 格式，时间均值用微秒（`ms * 1000 / iters`）。

> 若想真启用 FP，需要在 `trap/mod.rs` 里把 `sstatus.FS` 设为 Initial，
> 并保存/恢复浮点寄存器。Agent-OS 没有这么做，因为内核本身不需要 FP，
> 把它留作未来扩展。

### Q: 想看协议帧/Context Area 的字节细节

可以临时插一段 `println!("{:x?}", bytes)` 在 demo 里——所有
demo 用户态字节都拿得到。

---

## 五、目录速查

| 路径 | 内容 |
|---|---|
| `README.md` | 项目门面 |
| `docs/design/00-overview.md` | 总览 + mermaid 图 |
| `docs/design/01-protocol.md` | Tool Call 协议规格 |
| `docs/design/02-syscall-spec.md` | 19 个新 syscall（500–540） |
| `docs/adr/ADR-001-baseline-choice.md` | 为什么选 rCore-ch6 |
| `docs/adr/ADR-002-protocol-format.md` | 为什么选 postcard |
| `docs/adr/ADR-003-context-area-layout.md` | Context 区设计 |
| `docs/pitch.md` | **答辩讲稿**（1min / 5-10min / Q&A） |
| `docs/perf-report.md` | **性能报告**（含 113× 实测） |
| `docs/run-log.md` | QEMU 端到端运行记录 |
| `docs/impl-report-tool-call.md` | 零拷贝工具调用编码报告 |
| `docs/QUICKSTART.md` | 本文档 |
| `agent_proto/` | 共享协议 crate |
| `os/src/agent/` | 内核 Agent-OS 子系统 |
| `os/src/syscall/agent.rs` | 19 个新 syscall 入口 |
| `user/src/bin/agent_*` | 9 个 demo + agent_npc_worker + agent_runner |

---

## 六、清理 / 重建

```bash
# 完全重新构建
cd /home/xuliyang/tmp/os/agent-os
cd os && cargo clean
cd ../user && cargo clean
cd ../easy-fs && cargo clean
cd ../easy-fs-fuse && cargo clean

# 然后再 make run
cd ../os && make run
```
