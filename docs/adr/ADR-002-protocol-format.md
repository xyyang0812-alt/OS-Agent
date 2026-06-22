# ADR-002: 工具调用协议格式选择

**状态**：已采纳
**日期**：2026-06-22

## 上下文

任务二允许 JSON / 键值对 / 自定义二进制协议。选择直接影响：
- 性能（解析速度、内存占用）
- 创新性评分
- 调试便利度

## 决策

采用 **postcard 二进制编码 + 强类型枚举（`ToolName`）**。

```rust
// 帧头
#[repr(C)]
pub struct FrameHeader {
    magic: u32,    // 0xA9E47F00
    version: u16,
}
// 帧体由 postcard 序列化的 ToolRequest / ToolResponse 提供
```

## 理由

1. **性能**：postcard 在 RISC-V 64 上的解析速度约为 serde_json 的 8-15 倍（参考社区基准），且无堆分配（zero-copy 反序列化）。
2. **no_std 友好**：postcard 是为嵌入式设计的，不依赖 `std::string::String` 等。
3. **强类型**：`ToolName` 枚举编译期保证调用方与内核对工具集合的一致认知，比字符串匹配更安全。
4. **演进性**：postcard + serde 的 `#[serde(default)]` 支持字段级前向兼容。
5. **创新性**：比起"JSON over syscall"，这是更有 OS 工程美学的设计——评分文档强调"对 Agent 工作模式的深入理解"。

## 调试便利度的对冲

- 提供 `agent-trace` 用户态工具（任务六交付物之一），把二进制帧反编码为 JSON 文本，仅在调试时启用。
- 这样保留了**生产路径紧凑、调试路径可读**两全的格局。

## 替代方案与拒绝理由

| 替代 | 拒绝理由 |
|---|---|
| JSON | 解析慢、堆分配多、no_std 体验差 |
| Protobuf | 编译期外部依赖大，rCore 集成成本高 |
| Cap'n Proto / FlatBuffers | 同上，且对小消息不划算 |
| 纯 C struct | 缺乏版本演进能力 |
