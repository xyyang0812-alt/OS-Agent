//! Agent-OS 子系统入口模块
//!
//! 在 rCore 之上扩展面向 AI 智能体的内核功能：
//!
//! - [`pcb_ext`]: PCB 扩展字段 `AgentExt`
//! - [`context_area`]: 用户态 Agent Context 区分配与布局
//! - [`context_path`]: Agent Loop 上下文路径
//! - [`protocol`]: Tool Call 二进制协议（与用户态共享）
//! - [`tool`]: 内核工具集与调度器
//! - [`error`]: 统一错误类型
//!
//! 详细设计见 `docs/design/00-overview.md`。

// Agent 子系统的字段语义靠 docs/design 与 ADR 解释，
// 不在每个字段上重复写一遍 doc，故在子系统内部放开 missing_docs。
#![allow(missing_docs)]

pub mod blocking;
pub mod context_area;
pub mod context_path;
pub mod error;
pub mod event_bus;
pub mod file_attr;
pub mod pcb_ext;
pub mod protocol;
pub mod tool;

pub use context_area::AgentContextArea;
pub use error::{AgentError, AgentResult};
pub use pcb_ext::{AgentExt, AgentType, LoopState};
