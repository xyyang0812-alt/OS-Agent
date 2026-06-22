//! ToolDispatcher：把解码后的 `ToolRequest` 分发到具体工具
//!
//! 工具按强类型枚举 `ToolName` 分支。响应字节由各工具自己序列化（postcard）。

use alloc::vec::Vec;

use agent_proto::{ToolName, ToolRequest, ToolStatus};

use crate::agent::error::AgentResult;

use super::handlers;

/// 工具调度结果：成功时携带工具状态 + 已序列化的结果字节
pub struct DispatchResult {
    pub status: ToolStatus,
    pub body: Vec<u8>,
}

pub type ToolHandler = fn(req: &ToolRequest) -> AgentResult<DispatchResult>;

pub struct ToolDispatcher;

impl ToolDispatcher {
    pub fn dispatch(req: &ToolRequest) -> AgentResult<DispatchResult> {
        match req.tool {
            ToolName::SystemStatus => handlers::system_status(req),
            ToolName::QueryProcess => handlers::query_process(req),
            ToolName::ReadContext => handlers::read_context(req),
            ToolName::SendMessage => handlers::send_message(req),
            ToolName::QueryFile => handlers::query_file(req),
        }
    }

    /// 返回工具列表的描述（供 sys_tool_list 使用）。结果是已编码的 postcard 字节。
    pub fn list() -> Vec<u8> {
        use agent_proto::{QueryResult, ToolDescriptor};
        use alloc::string::ToString;
        let items: Vec<ToolDescriptor> = ToolName::ALL
            .iter()
            .map(|t| ToolDescriptor {
                name: t.as_str().to_string(),
                doc: tool_doc(*t).to_string(),
            })
            .collect();
        let wrapped = QueryResult { items };
        postcard::to_allocvec(&wrapped).unwrap_or_default()
    }
}

fn tool_doc(t: ToolName) -> &'static str {
    match t {
        ToolName::SystemStatus => "Returns overall system status (proc count, memory, uptime).",
        ToolName::QueryProcess => "Filter processes by status / agent type.",
        ToolName::ReadContext => "Read structured info about a process / file / agent by id.",
        ToolName::SendMessage => "Send an opaque byte payload to another process (task-5).",
        ToolName::QueryFile => "Query files by tag / owner / keyword (task-4).",
    }
}
