//! Agent 进程的 PCB 扩展字段
//!
//! 这些字段挂在 `TaskControlBlockInner` 的 `agent_ext: Option<Box<AgentExt>>`，
//! 普通进程为 `None`，对它们零开销。

use alloc::boxed::Box;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::mm::VirtAddr;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentType {
    /// 普通 Agent 进程（与普通进程的区别是有 Context 区）
    Normal,
    /// 系统级 Agent（如系统管理员 Agent，特权更高）
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LoopState {
    Idle,
    Thinking,
    Calling,
    Observing,
    Done,
}

/// Context Path 节点元信息（数据本体存在用户态 Context Area Path Buffer）
#[derive(Debug, Clone, Copy)]
pub struct PathNodeMeta {
    /// 该节点在 Path Buffer 中的字节偏移
    pub offset_in_buffer: u32,
    /// 该节点占用的字节数
    pub len: u32,
    /// 写入时的时钟（用于 LRU 淘汰）
    pub write_time: u64,
    /// 节点序号（单调递增）
    pub seq: u64,
}

/// 路径配额与策略
#[derive(Debug, Clone, Copy)]
pub struct PathQuota {
    /// 路径最多节点数
    pub max_nodes: u32,
    /// 路径最多占用字节数（Path Buffer 内）
    pub max_bytes: u32,
    /// 淘汰策略
    pub policy: EvictPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictPolicy {
    Fifo,
    Lru,
}

/// Agent 进程在 PCB 中的扩展
pub struct AgentExt {
    pub agent_type: AgentType,
    pub heartbeat_interval_ms: u64,
    pub loop_state: LoopState,
    /// Agent Context 区在用户地址空间中的起始虚拟地址
    pub context_area_base: VirtAddr,
    /// Agent Context 区大小（字节，按页对齐）
    pub context_area_size: usize,
    /// Context Path 的元信息列表（实际数据在用户态）
    pub path_meta: Vec<PathNodeMeta>,
    /// Context Path 配额
    pub path_quota: PathQuota,
    /// 已使用字节数（在 Path Buffer 中）
    pub path_used_bytes: u32,
    /// 节点序号生成器
    pub next_seq: u64,
    // ---- 任务五：Loop 运行时 ----
    /// 上一次心跳触发的时间戳（毫秒）
    pub heartbeat_last_ms: u64,
    /// 心跳是否到期未消费
    pub heartbeat_pending: bool,
    /// 文件变更事件是否到达未消费（任务五 EVENT_FILE_MODIFIED）
    pub file_event_pending: bool,
    /// 关注的事件位掩码（参见 `event_bus` 模块的常量）
    pub watched_events: u32,
    /// 消息收件箱：每条消息一个 Vec<u8>
    pub mailbox: Vec<Vec<u8>>,
    /// 邮箱中最多保留多少条消息（超过则丢弃最旧的）
    pub mailbox_capacity: usize,
}

impl AgentExt {
    /// 默认配置
    pub fn new(
        agent_type: AgentType,
        context_area_base: VirtAddr,
        context_area_size: usize,
        max_nodes: u32,
        max_bytes: u32,
    ) -> Box<Self> {
        Box::new(Self {
            agent_type,
            heartbeat_interval_ms: 0,
            loop_state: LoopState::Idle,
            context_area_base,
            context_area_size,
            path_meta: Vec::new(),
            path_quota: PathQuota {
                max_nodes,
                max_bytes,
                policy: EvictPolicy::Lru,
            },
            path_used_bytes: 0,
            next_seq: 0,
            heartbeat_last_ms: 0,
            heartbeat_pending: false,
            file_event_pending: false,
            watched_events: 0,
            mailbox: Vec::new(),
            mailbox_capacity: 32,
        })
    }
}
