# Tool Call 协议设计

## 1. 设计取舍

| 候选 | 优点 | 缺点 | 结论 |
|---|---|---|---|
| JSON（serde_json） | 通用、可读 | 解析慢、堆分配多、字符串处理在 no_std 较重 | 不选 |
| Protobuf | 强类型 + 紧凑 | 编译期依赖 .proto，生态在 no_std 偏重 | 不选 |
| 纯 C struct（FFI） | 零开销 | 不可扩展、版本难管理 | 不选 |
| **postcard**（serde + 紧凑二进制） | no_std 友好、零拷贝反序列化、强类型、变长编码紧凑 | 不可读（但 Trace 工具可解决） | **选** |

详见 [`../adr/ADR-002-protocol-format.md`](../adr/ADR-002-protocol-format.md)。

## 2. 帧格式

```
┌────────────┬────────────┬─────────────────────────────────┐
│ MagicHdr 4 │ Version 2  │     postcard body (variable)    │
└────────────┴────────────┴─────────────────────────────────┘
   0xA9E47F00     0x0001
```

- **MagicHdr**：识别非 Agent 协议帧
- **Version**：协议演进时增量
- **Body**：postcard 编码的 `ToolRequest` 或 `ToolResponse`

## 3. 请求类型

```rust
pub struct ToolRequest<'a> {
    pub req_id: u64,            // 关联请求/响应
    pub tool: ToolName,         // 强类型枚举（非字符串）
    pub params: ToolParams<'a>,
}

pub enum ToolName {
    QueryProcess,
    QueryFile,
    ReadContext,
    SendMessage,
    SystemStatus,
}

pub enum ToolParams<'a> {
    QueryProcess { status: Option<ProcStatus>, ty: Option<AgentType> },
    QueryFile    { tag: Option<&'a str>, owner: Option<&'a str>, keyword: Option<&'a str> },
    ReadContext  { target_type: ContextTargetType, target_id: u64 },
    SendMessage  { target_pid: usize, payload: &'a [u8] },
    SystemStatus,
}
```

**为什么用枚举而非字符串作为工具名**？
- 编译期捕获拼写错误
- 反序列化更快（单字节 tag）
- 工具集合是**有限闭集**，正合枚举语义

## 4. 响应类型

```rust
pub struct ToolResponse {
    pub req_id: u64,
    pub status: ToolStatus,
    /// 结果数据在 Agent Context 区中的位置
    /// 若结果为空，则 result_len = 0
    pub result_offset: u32,
    pub result_len: u32,
}

pub enum ToolStatus {
    Ok,
    ToolNotFound,
    BadParams,
    PermissionDenied,
    QuotaExceeded,
    InternalError,
}
```

**关键点**：响应本身只有 24 字节左右，真正的结果体由内核**直接写入用户空间共享内存**。用户态拿到 `(offset, len)` 后**无 syscall 读取**。

## 5. 错误处理

- syscall 返回值约定：`>=0` 为成功，`<0` 为内核级错误（参数指针非法、协议帧损坏等）
- 工具级错误（参数错、权限不足）走 `ToolStatus`，syscall 仍返回成功

## 6. 演进规则

- 新增工具 → 给 `ToolName` 加 variant，旧客户端遇到新 variant 时返回 `ToolNotFound`
- 修改参数 → 在 `ToolParams` variant 中**只能加字段不能改字段**，加 `#[serde(default)]`
- 不兼容变更 → 提升协议 `Version`
