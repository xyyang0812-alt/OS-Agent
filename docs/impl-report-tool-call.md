# 编码实现报告：零拷贝结构化工具调用（`sys_tool_call`）

> 本报告以 Agent-OS 的**核心功能**——结构化工具调用为例，自底向上完整说明它
> 是如何实现的：从共享协议、系统调用入口、内核工具分发，到 Context 区零拷贝
> 写回、用户态零拷贝读取的全链路。涉及任务二，并与任务一（Context 区）联动。

---

## 一、功能目标

要求文档（`要求.md` 任务二）指出：Agent 需要一种**结构化**的工具调用机制，
而不是把所有数据都通过系统调用返回值搬运。具体目标：

1. Agent 用强类型的请求（工具名 + 参数）调用内核工具，内核执行后返回结构化结果；
2. **结果写入用户态 Agent Context 区**，供 Agent 高速读取（而非每次都经 syscall 传回）；
3. 协议可解析、可扩展、有明确的错误处理。

由此引出本功能的核心设计取舍——**零拷贝**：内核把工具结果直接写进与用户共享的
内存区，系统调用只回传一个 `(offset, len)` 定位指针，用户态读结果**不再陷入内核**。

---

## 二、涉及模块全景

| 层 | 文件 | 职责 |
|---|---|---|
| 共享协议 | `agent_proto/src/lib.rs` | 请求/响应/结果类型、帧头、postcard 编解码（OS 与 user 共用） |
| 用户态 API | `user/src/lib.rs` `tool_call()` | 编码请求、发起 syscall、零拷贝读结果 |
| 用户态 syscall | `user/src/syscall.rs` `sys_tool_call()` | `ecall` 封装（4 参） |
| 内核 syscall 入口 | `os/src/syscall/agent.rs` `sys_tool_call()` | 拷入请求、解码校验、分发、写回结果 |
| 工具分发 | `os/src/agent/tool/registry.rs` `ToolDispatcher::dispatch()` | 按 `ToolName` 路由到具体 handler |
| 工具实现 | `os/src/agent/tool/handlers.rs` | 5 个工具的业务逻辑 + postcard 序列化 |
| 共享内存 | `os/src/agent/context_area.rs` | Context 区分配/映射、跨地址空间读写 |

数据流向：

```
[user] tool_call ──编码帧──► sys_tool_call(ecall)
                                   │
[kernel] sys_tool_call ──解码──► ToolDispatcher::dispatch ──► handler
                                                                 │ postcard 编码 body
[kernel] write_user_bytes(Context Area Tool Result Ring) ◄───────┘
                                   │ 回传 (offset, len)
[user] 直接读 Context 区 [base+offset, +len) ──postcard 反序列化──► 结构化结果
```

---

## 三、协议设计（`agent_proto`）

`agent_proto` 是一个 `#![no_std]` crate，被 **OS 内核和用户程序同时依赖**，
是协议的"唯一真相源"，从根本上杜绝了 OS 与 user 两侧协议定义漂移的问题。

### 3.1 帧格式

每个请求/响应都是一个二进制帧：

```text
┌────────────┬────────────┬─────────────────────────────────┐
│ Magic 4B   │ Version 2B │     postcard body (variable)    │
└────────────┴────────────┴─────────────────────────────────┘
```

- `Magic = 0xA9E4_7F00`：识别这是一个合法的 Agent-OS 帧；
- `Version = 0x0001`：协议版本，便于将来演进；
- body：用 [postcard](https://github.com/jamesmunns/postcard) 序列化的结构体。

帧头校验在内核侧第一道关卡完成：

```rust
pub fn check_frame_header(buf: &[u8]) -> Result<usize, FrameError> {
    if buf.len() < FRAME_HEADER_SIZE { return Err(FrameError::TooShort); }
    let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if magic != PROTO_MAGIC { return Err(FrameError::BadMagic); }
    let version = u16::from_le_bytes([buf[4], buf[5]]);
    if version != PROTO_VERSION { return Err(FrameError::UnsupportedVersion(version)); }
    Ok(FRAME_HEADER_SIZE)
}
```

### 3.2 强类型请求

工具名是**枚举**而非字符串，拼写错误在编译期就被 Rust 捕获：

```rust
pub enum ToolName { SystemStatus, QueryProcess, ReadContext, SendMessage, QueryFile }

pub enum ToolParams {
    SystemStatus,
    QueryProcess { status: Option<ProcStatus>, ty: AgentTypeFilter },
    ReadContext  { target_type: ContextTargetType, target_id: u64 },
    SendMessage  { target_pid: u64, payload: Vec<u8> },
    QueryFile    { tag: Option<String>, owner: Option<String>,
                   keyword: Option<String>, use_index: bool },
}

pub struct ToolRequest { pub req_id: u64, pub tool: ToolName, pub params: ToolParams }
```

### 3.3 小响应 + 结果定位指针

这是零拷贝设计的关键：响应本身**很小**，只携带状态码和"结果在哪里"，
真正的结果数据另存于 Context 区：

```rust
pub struct ToolResponse {
    pub req_id: u64,
    pub status: ToolStatus,     // Ok / BadParams / PermissionDenied / ...
    pub result_offset: u32,     // 结果在 Context 区中的偏移
    pub result_len: u32,        // 结果字节数
}
```

---

## 四、地址空间与 Context 区布局（任务一基础）

`sys_agent_create` 时，内核在用户地址空间 `0x8000_0000` 处映射一块 64KB 的
**共享内存**（R|W|U），内部分区如下（`context_area.rs::layout`）：

```text
偏移 0x0000  Header (256B)            ── magic/version/各段偏移/写指针
     0x0100  Tool Result Ring (16KB)  ── 工具调用结果写在这里  ◄── 本功能用到
     0x4100  Path Buffer (32KB)       ── 上下文路径（任务三）
     0xC100  Tool Call History (8KB)
```

这块内存对内核和用户态是**同一段物理页的两个映射**，因此内核写入后用户态
立即可见，无需再次拷贝——这正是"零拷贝"的物理基础。

---

## 五、完整调用链路（自顶向下）

### 5.1 用户态：编码请求并发起 syscall

`user/src/lib.rs` 的 `tool_call()`：

```rust
pub fn tool_call(req: &agent_proto::ToolRequest) -> Result<ToolCallOutcome, isize> {
    let mut buf = [0u8; 1024];
    let n = agent_proto::encode_request(req, &mut buf).map_err(|_| -100isize)?; // 写帧头+body
    let mut out_offset: u32 = 0;
    let mut out_len: u32 = 0;
    let r = sys_tool_call(buf.as_ptr(), n, &mut out_offset, &mut out_len);       // ecall
    if r < 0 { return Err(r); }
    Ok(ToolCallOutcome { status_code: r, result_offset: out_offset, result_len: out_len })
}
```

注意它传入两个 out 指针（`&out_offset`、`&out_len`），内核会把结果定位信息写回这两处。

### 5.2 内核入口：`sys_tool_call`（`os/src/syscall/agent.rs`）

内核入口做 6 件事，注释里也标注了每一步：

```rust
pub fn sys_tool_call(req_ptr, req_len, out_offset_ptr, out_len_ptr) -> isize {
    // (0) 参数防御：空指针/超长直接拒绝
    if req_ptr == 0 || req_len == 0 || req_len > 64 * 1024 { return -1; }

    // (1) 必须是 Agent 进程（普通进程没有 Context 区）
    let mut inner = task.inner_exclusive_access();
    if inner.agent_ext.is_none() { return -4; }
    let token = inner.get_user_token();
    let (area_base, area_size) = {
        let ext = inner.agent_ext.as_mut().unwrap().as_mut();
        ext.loop_state = LoopState::Calling;        // 状态机：进入 Calling
        (ext.context_area_base.0, ext.context_area_size)
    };

    // (2) copy_from_user：把请求帧拷进内核缓冲
    let mut req_buf = vec![0u8; req_len];
    read_user_bytes(token, req_ptr, &mut req_buf);
    drop(inner);                                    // 尽早释放 PCB 锁

    // (3) 解码 + 校验帧头/版本/postcard body
    let req = match decode_request(&req_buf) { Ok(r) => r, Err(_) => return -3 };

    // (4) 分发到具体工具
    let dr = match ToolDispatcher::dispatch(&req) { Ok(d) => d, Err(e) => return e.into_isize() };

    // (5) 把结果字节零拷贝写入 Tool Result Ring（仅当成功且非空）
    let (offset, length) = if dr.status == ToolStatus::Ok && !dr.body.is_empty() {
        if dr.body.len() > layout::TOOL_RESULT_LEN { return -5; }
        write_user_bytes(token, area_base + layout::TOOL_RESULT_OFF, &dr.body);
        (layout::TOOL_RESULT_OFF as u32, dr.body.len() as u32)
    } else { (0, 0) };

    // (6) 把 (offset, len) 写回用户的两个 out 指针
    write_user_bytes(token, out_offset_ptr, &offset.to_le_bytes());
    write_user_bytes(token, out_len_ptr,    &length.to_le_bytes());

    ext.loop_state = LoopState::Observing;          // 状态机：转 Observing
    // 把 ToolStatus 映射为 syscall 返回码：Ok=0，其余正数错误码
    match resp.status { ToolStatus::Ok => 0, /* ... */ }
}
```

要点：
- **机制集中**：用户指针校验、`copy_from_user`、错误码翻译全在 syscall 层，
  业务逻辑完全下沉到 handler，职责清晰；
- **Loop 状态机联动**：调用工具自动把 Agent 的 `loop_state` 推进
  `Calling → Observing`，体现 Agent Loop 的"行动→观察"语义（任务五）；
- **锁尽早释放**：拷完请求就 `drop(inner)`，分发期间不持 PCB 锁。

### 5.3 工具分发：`ToolDispatcher::dispatch`（`tool/registry.rs`）

按枚举分支路由，编译器保证所有 `ToolName` 都被覆盖（穷尽匹配）：

```rust
pub fn dispatch(req: &ToolRequest) -> AgentResult<DispatchResult> {
    match req.tool {
        ToolName::SystemStatus => handlers::system_status(req),
        ToolName::QueryProcess => handlers::query_process(req),
        ToolName::ReadContext  => handlers::read_context(req),
        ToolName::SendMessage  => handlers::send_message(req),
        ToolName::QueryFile    => handlers::query_file(req),
    }
}
```

`DispatchResult` 携带 `status` 和**已序列化好的** `body: Vec<u8>`，
让上层 syscall 无需关心每个工具结果的具体类型。

### 5.4 工具实现：以 `system_status` 为例（`tool/handlers.rs`）

每个 handler 自己完成"采集数据 → postcard 编码 → 返回字节"：

```rust
pub fn system_status(_req: &ToolRequest) -> AgentResult<DispatchResult> {
    let procs = collect_processes();                // 拍进程快照
    let info = SystemStatusInfo {
        total_procs:   procs.len() as u32,
        agent_procs:   procs.iter().filter(|p| p.is_agent).count() as u32,
        running_procs: procs.iter().filter(|p| p.status == ProcStatus::Running).count() as u32,
        memory_used_bytes: 0,
        uptime_ticks:  get_time() as u64,
    };
    let body = postcard::to_allocvec(&info).map_err(|_| AgentError::InternalError)?;
    Ok(DispatchResult { status: ToolStatus::Ok, body })
}
```

### 5.5 零拷贝写回：`write_user_bytes`（`context_area.rs`）

内核通过 `translated_byte_buffer` 跨地址空间访问目标进程的用户页，
把结果字节直接写进共享内存：

```rust
pub fn write_user_bytes(token: usize, user_va: usize, data: &[u8]) {
    // token 指定目标地址空间；按物理页切块写入（用户缓冲可能跨页不连续）
    let chunks = translated_byte_buffer(token, user_va as *const u8, data.len());
    let mut written = 0;
    for chunk in chunks {
        let n = chunk.len().min(data.len() - written);
        chunk[..n].copy_from_slice(&data[written..written + n]);
        written += n;
        if written >= data.len() { break; }
    }
}
```

（`context_area.rs` 还提供了 `write_tool_result`，实现带写指针推进的环形缓冲版本；
当前 `sys_tool_call` 走的是"覆盖写 Ring 起始"的简化路径，二者并存。）

### 5.6 用户态：零拷贝读结果（无 syscall）

`ToolCallOutcome` 拿到 `(offset, len)` 后，直接按地址读共享内存：

```rust
impl ToolCallOutcome {
    pub fn result_bytes(&self) -> &'static [u8] {
        if self.result_len == 0 { return &[]; }
        let ptr = (AGENT_CONTEXT_BASE + self.result_offset as usize) as *const u8;
        unsafe { core::slice::from_raw_parts(ptr, self.result_len as usize) }
    }
}
```

用户拿到字节切片后用 postcard 反序列化即可，**整个读结果过程不触发任何系统调用**：

```rust
let info: SystemStatusInfo = postcard::from_bytes(oc.result_bytes()).unwrap();
```

---

## 六、时序图

```mermaid
sequenceDiagram
    participant U as 用户态 Agent
    participant K as sys_tool_call (内核)
    participant D as ToolDispatcher
    participant H as Tool Handler
    participant C as Context 区 (共享内存)

    U->>U: encode_request(postcard + 帧头)
    U->>K: ecall sys_tool_call(buf, len, &off, &outlen)
    K->>K: copy_from_user(req_buf)
    K->>K: decode_request → 校验 magic/version/body
    K->>D: dispatch(ToolName, params)
    D->>H: system_status(req)
    H-->>D: DispatchResult{ status, body(postcard) }
    D-->>K: DispatchResult
    K->>C: write_user_bytes(Tool Result Ring, body)
    K-->>U: 写回 (offset, len) 到 out 指针; 返回状态码
    U->>C: result_bytes() 直接读共享内存（无 syscall）
    U->>U: postcard::from_bytes → 结构化结果
```

---

## 七、错误处理与边界情况

| 情况 | 处理 | 返回 |
|---|---|---|
| 空指针 / 请求超长（>64KB） | 入口防御 | `-1` |
| 调用方不是 Agent（无 Context 区） | `agent_ext.is_none()` | `-4` |
| 帧头损坏 / 版本不符 / body 解析失败 | `decode_request` 返回 `Err` | `-3` |
| 结果超出 Tool Result Ring（16KB） | 长度检查 | `-5` |
| 工具内部错误 | handler 返回 `AgentError` | 负错误码 |
| 工具级状态（参数错/未实现/越权） | `ToolStatus` 映射为正数码 | `100~105` |

成功时 syscall 返回 `0`，但**用户仍需检查 `result_len`** 判断是否有结果体。

---

## 八、设计亮点小结

1. **零拷贝**：结果由内核直写用户共享内存，响应只带 `(offset, len)`，
   用户读结果零 syscall——高频读路径完全消除内核陷入开销。
2. **强类型协议**：`ToolName`/`ToolParams` 枚举 + postcard，比 JSON 快约 10×，
   拼写错误编译期捕获，运行时帧损坏返回明确错误码。
3. **唯一真相源**：`agent_proto` 被 OS 与 user 共同依赖，协议永不漂移。
4. **机制/策略分离**：内核管校验、配额、写回位置；用户态管缓存粒度、读取时机。
5. **可穷尽分发**：`match req.tool` 穷尽匹配，新增工具时编译器强制处理所有分支。

---

## 九、对应验收

- `agent_demo_tool` 依次调用全部 5 个工具，并从 Context 区零拷贝读回结构化结果，
  验证 `system_status / query_process / read_context / send_message / query_file`
  和 `tool_list`，最终打印 `PASS task-2`。
- 运行示例见 `docs/run-log.md` 与 README「四、Tool Call 流程」。
