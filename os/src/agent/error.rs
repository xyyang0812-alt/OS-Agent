//! Agent-OS 统一错误类型

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentError {
    /// 用户指针非法 / copy_from_user 失败
    BadUserPointer,
    /// 用户提供的缓冲区不足
    BufferTooSmall,
    /// 协议帧损坏
    ProtocolMalformed,
    /// 当前进程不是 Agent，未先 `sys_agent_create`
    NotAnAgent,
    /// 资源配额耗尽
    QuotaExceeded,
    /// 目标不存在（pid、上下文节点、watch_id 等）
    NotFound,
    /// 权限不足
    PermissionDenied,
    /// 工具内部错误
    InternalError,
}

pub type AgentResult<T> = Result<T, AgentError>;

impl AgentError {
    /// 转换为 syscall 返回值约定
    pub fn into_isize(self) -> isize {
        match self {
            Self::BadUserPointer => -1,
            Self::BufferTooSmall => -2,
            Self::ProtocolMalformed => -3,
            Self::NotAnAgent => -4,
            Self::QuotaExceeded => -5,
            Self::NotFound => -6,
            Self::PermissionDenied => -7,
            Self::InternalError => -8,
        }
    }
}

// 工具层面的状态枚举 ToolStatus 现在定义在 `agent_proto` crate，
// 通过 `crate::agent::protocol::ToolStatus` 访问。

