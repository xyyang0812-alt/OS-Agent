//! Tool Call 协议（内核侧入口）
//!
//! 真正的类型定义在 [`agent_proto`] crate 中，OS 与 user 都依赖它，
//! 保证唯一的真相源。本模块仅做 re-export，便于在内核代码里以
//! `crate::agent::protocol::ToolRequest` 这样的短路径访问。

pub use agent_proto::*;
