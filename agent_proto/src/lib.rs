//! Agent-OS Tool Call 协议（OS 与 user 共享的唯一真相源）
//!
//! 帧格式：
//!
//! ```text
//! ┌────────────┬────────────┬─────────────────────────────────┐
//! │ Magic 4B   │ Version 2B │     postcard body (variable)    │
//! └────────────┴────────────┴─────────────────────────────────┘
//! ```
//!
//! 详细背景与决策见 `docs/design/01-protocol.md`。

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

pub const PROTO_MAGIC: u32 = 0xA9E4_7F00;
pub const PROTO_VERSION: u16 = 0x0001;
/// 帧头总长度（magic 4B + version 2B）
pub const FRAME_HEADER_SIZE: usize = 6;

/// 写入帧头到给定缓冲区，返回写入字节数。缓冲区必须 >= FRAME_HEADER_SIZE。
pub fn write_frame_header(buf: &mut [u8]) -> usize {
    assert!(buf.len() >= FRAME_HEADER_SIZE);
    buf[0..4].copy_from_slice(&PROTO_MAGIC.to_le_bytes());
    buf[4..6].copy_from_slice(&PROTO_VERSION.to_le_bytes());
    FRAME_HEADER_SIZE
}

/// 校验帧头并返回 body 起始偏移
pub fn check_frame_header(buf: &[u8]) -> Result<usize, FrameError> {
    if buf.len() < FRAME_HEADER_SIZE {
        return Err(FrameError::TooShort);
    }
    let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if magic != PROTO_MAGIC {
        return Err(FrameError::BadMagic);
    }
    let version = u16::from_le_bytes([buf[4], buf[5]]);
    if version != PROTO_VERSION {
        return Err(FrameError::UnsupportedVersion(version));
    }
    Ok(FRAME_HEADER_SIZE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    TooShort,
    BadMagic,
    UnsupportedVersion(u16),
}

/// 工具名（强类型枚举 → 编译期捕获拼写错误）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolName {
    /// 获取系统整体状态
    SystemStatus,
    /// 按条件查询进程
    QueryProcess,
    /// 读取指定对象的结构化信息
    ReadContext,
    /// 向其他进程发消息（任务五配合事件总线）
    SendMessage,
    /// 按属性查询文件（任务四完整实现）
    QueryFile,
}

impl ToolName {
    pub const ALL: &'static [ToolName] = &[
        ToolName::SystemStatus,
        ToolName::QueryProcess,
        ToolName::ReadContext,
        ToolName::SendMessage,
        ToolName::QueryFile,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ToolName::SystemStatus => "system_status",
            ToolName::QueryProcess => "query_process",
            ToolName::ReadContext => "read_context",
            ToolName::SendMessage => "send_message",
            ToolName::QueryFile => "query_file",
        }
    }
}

/// 进程状态过滤
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProcStatus {
    Running,
    Ready,
    Zombie,
    /// Agent 主动 `sys_agent_wait` 进入真休眠（不在 ready queue 中）
    Blocked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentTypeFilter {
    Any,
    Normal,
    Agent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContextTargetType {
    Process,
    File,
    Agent,
}

/// 工具参数（每个工具一个 variant）
#[derive(Debug, Serialize, Deserialize)]
pub enum ToolParams {
    SystemStatus,
    QueryProcess {
        status: Option<ProcStatus>,
        ty: AgentTypeFilter,
    },
    ReadContext {
        target_type: ContextTargetType,
        target_id: u64,
    },
    SendMessage {
        target_pid: u64,
        payload: Vec<u8>,
    },
    QueryFile {
        tag: Option<String>,
        owner: Option<String>,
        keyword: Option<String>,
        /// 用 false 走全量扫描（任务四性能对比的对照组），默认 true 走索引
        use_index: bool,
    },
}

/// 工具调用请求
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolRequest {
    pub req_id: u64,
    pub tool: ToolName,
    pub params: ToolParams,
}

/// 工具状态
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolStatus {
    Ok,
    ToolNotFound,
    BadParams,
    PermissionDenied,
    QuotaExceeded,
    InternalError,
    NotImplemented,
}

/// 工具调用响应（小：只携带定位指针 + 状态）
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolResponse {
    pub req_id: u64,
    pub status: ToolStatus,
    pub result_offset: u32,
    pub result_len: u32,
}

// ============ 结果数据类型 ============

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub status: ProcStatus,
    pub is_agent: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfo {
    pub inode_id: u32,
    pub name: String,
    pub size: u32,
    pub tags: Vec<String>,
    pub owner: Option<String>,
    pub digest_preview: String,
}

/// 任务四性能对比专用：把"用索引"与"全量扫描"两种路径都跑一遍，分别返回
/// 耗时和结果集，让用户态对比。
#[derive(Debug, Serialize, Deserialize)]
pub struct QueryFileBenchmark {
    pub indexed_us: u64,
    pub scan_us: u64,
    pub indexed_results: Vec<String>,
    pub scan_results: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemStatusInfo {
    pub total_procs: u32,
    pub agent_procs: u32,
    pub running_procs: u32,
    pub memory_used_bytes: u64,
    pub uptime_ticks: u64,
}

/// 多条结果的通用包裹
#[derive(Debug, Serialize, Deserialize)]
pub struct QueryResult<T> {
    pub items: Vec<T>,
}

/// 用户态收到的工具列表条目
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub doc: String,
}

// ============ 便捷构造 / 编解码 ============

/// 把请求序列化成完整帧（含帧头）到 `out` 缓冲，返回写入字节数。
pub fn encode_request(req: &ToolRequest, out: &mut [u8]) -> Result<usize, postcard::Error> {
    if out.len() < FRAME_HEADER_SIZE {
        return Err(postcard::Error::SerializeBufferFull);
    }
    write_frame_header(out);
    let body = postcard::to_slice(req, &mut out[FRAME_HEADER_SIZE..])?;
    Ok(FRAME_HEADER_SIZE + body.len())
}

/// 把响应序列化成完整帧（含帧头）。
pub fn encode_response(resp: &ToolResponse, out: &mut [u8]) -> Result<usize, postcard::Error> {
    if out.len() < FRAME_HEADER_SIZE {
        return Err(postcard::Error::SerializeBufferFull);
    }
    write_frame_header(out);
    let body = postcard::to_slice(resp, &mut out[FRAME_HEADER_SIZE..])?;
    Ok(FRAME_HEADER_SIZE + body.len())
}

#[derive(Debug)]
pub enum DecodeError {
    Frame(FrameError),
    Postcard(postcard::Error),
}

impl From<FrameError> for DecodeError {
    fn from(e: FrameError) -> Self {
        DecodeError::Frame(e)
    }
}
impl From<postcard::Error> for DecodeError {
    fn from(e: postcard::Error) -> Self {
        DecodeError::Postcard(e)
    }
}

pub fn decode_request(buf: &[u8]) -> Result<ToolRequest, DecodeError> {
    let body_off = check_frame_header(buf)?;
    let req: ToolRequest = postcard::from_bytes(&buf[body_off..])?;
    Ok(req)
}

pub fn decode_response(buf: &[u8]) -> Result<ToolResponse, DecodeError> {
    let body_off = check_frame_header(buf)?;
    let resp: ToolResponse = postcard::from_bytes(&buf[body_off..])?;
    Ok(resp)
}
