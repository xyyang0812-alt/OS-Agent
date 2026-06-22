//! 五个内核工具的实现
//!
//! 工具调用约定：成功路径返回 `DispatchResult { status, body }`，
//! 其中 `body` 是已 postcard 序列化的结果字节，由 `sys_tool_call`
//! 负责写入用户态 Context Area Tool Result Ring。

use alloc::string::ToString;
use alloc::vec::Vec;

use agent_proto::{
    ContextTargetType, FileInfo, ProcStatus, ProcessInfo, QueryResult, SystemStatusInfo,
    ToolParams, ToolRequest, ToolStatus,
};

use crate::agent::error::AgentResult;
use crate::agent::file_attr::FILE_ATTR_STORE;
use crate::task::{TaskStatus, snapshot_all_processes};
use crate::timer::get_time;

use super::registry::DispatchResult;

/// 把 rCore 的 `TaskStatus` 转换为 wire 协议的 `ProcStatus`
fn map_status(s: TaskStatus) -> ProcStatus {
    match s {
        TaskStatus::Ready => ProcStatus::Ready,
        TaskStatus::Running => ProcStatus::Running,
        TaskStatus::Zombie => ProcStatus::Zombie,
        TaskStatus::Blocked => ProcStatus::Blocked,
    }
}

/// 收集系统所有进程的快照信息（供多个工具复用）。
fn collect_processes() -> Vec<ProcessInfo> {
    let tasks = snapshot_all_processes();
    let mut infos = Vec::with_capacity(tasks.len());
    for t in tasks {
        let pid = t.getpid() as u32;
        let inner = t.inner_exclusive_access();
        // ch6 的 TCB 没有 name 字段，这里用 pid 占位
        infos.push(ProcessInfo {
            pid,
            name: alloc::format!("proc-{}", pid),
            status: map_status(inner.task_status),
            is_agent: inner.agent_ext.is_some(),
        });
    }
    infos
}

/// system_status
pub fn system_status(_req: &ToolRequest) -> AgentResult<DispatchResult> {
    let procs = collect_processes();
    let total = procs.len() as u32;
    let agents = procs.iter().filter(|p| p.is_agent).count() as u32;
    let running = procs
        .iter()
        .filter(|p| p.status == ProcStatus::Running)
        .count() as u32;

    let info = SystemStatusInfo {
        total_procs: total,
        agent_procs: agents,
        running_procs: running,
        memory_used_bytes: 0, // TODO: 从 frame_allocator 拿
        uptime_ticks: get_time() as u64,
    };
    let body = postcard::to_allocvec(&info).map_err(|_| {
        crate::agent::error::AgentError::InternalError
    })?;
    Ok(DispatchResult {
        status: ToolStatus::Ok,
        body,
    })
}

/// query_process
pub fn query_process(req: &ToolRequest) -> AgentResult<DispatchResult> {
    let (status_filter, ty_filter) = match &req.params {
        ToolParams::QueryProcess { status, ty } => (*status, *ty),
        _ => {
            return Ok(DispatchResult {
                status: ToolStatus::BadParams,
                body: Vec::new(),
            });
        }
    };

    let mut procs = collect_processes();
    procs.retain(|p| {
        let status_ok = match status_filter {
            None => true,
            Some(s) => p.status == s,
        };
        let ty_ok = match ty_filter {
            agent_proto::AgentTypeFilter::Any => true,
            agent_proto::AgentTypeFilter::Agent => p.is_agent,
            agent_proto::AgentTypeFilter::Normal => !p.is_agent,
        };
        status_ok && ty_ok
    });

    let body = postcard::to_allocvec(&QueryResult { items: procs })
        .map_err(|_| crate::agent::error::AgentError::InternalError)?;
    Ok(DispatchResult {
        status: ToolStatus::Ok,
        body,
    })
}

/// read_context
pub fn read_context(req: &ToolRequest) -> AgentResult<DispatchResult> {
    let (target_type, target_id) = match &req.params {
        ToolParams::ReadContext {
            target_type,
            target_id,
        } => (*target_type, *target_id),
        _ => {
            return Ok(DispatchResult {
                status: ToolStatus::BadParams,
                body: Vec::new(),
            });
        }
    };

    match target_type {
        ContextTargetType::Process | ContextTargetType::Agent => {
            let procs = collect_processes();
            if let Some(p) = procs.into_iter().find(|p| p.pid as u64 == target_id) {
                if target_type == ContextTargetType::Agent && !p.is_agent {
                    return Ok(DispatchResult {
                        status: ToolStatus::PermissionDenied,
                        body: Vec::new(),
                    });
                }
                let body = postcard::to_allocvec(&p)
                    .map_err(|_| crate::agent::error::AgentError::InternalError)?;
                Ok(DispatchResult {
                    status: ToolStatus::Ok,
                    body,
                })
            } else {
                Ok(DispatchResult {
                    status: ToolStatus::ToolNotFound, // 复用 ToolNotFound 表示 "not found"
                    body: Vec::new(),
                })
            }
        }
        ContextTargetType::File => Ok(DispatchResult {
            status: ToolStatus::NotImplemented,
            body: Vec::new(),
        }),
    }
}

/// send_message —— 任务五：把 payload 投递到目标 Agent 的邮箱
pub fn send_message(req: &ToolRequest) -> AgentResult<DispatchResult> {
    let (target_pid, payload) = match &req.params {
        ToolParams::SendMessage {
            target_pid,
            payload,
        } => (*target_pid as usize, payload.clone()),
        _ => {
            return Ok(DispatchResult {
                status: ToolStatus::BadParams,
                body: Vec::new(),
            });
        }
    };
    let ok = crate::agent::event_bus::deliver_message(target_pid, payload);
    Ok(DispatchResult {
        status: if ok {
            ToolStatus::Ok
        } else {
            ToolStatus::PermissionDenied
        },
        body: Vec::new(),
    })
}

/// query_file —— 任务四的核心：按 tag/owner/keyword 走属性索引，
/// 返回 `QueryResult<FileInfo>`。
pub fn query_file(req: &ToolRequest) -> AgentResult<DispatchResult> {
    let (tag, owner, keyword, use_index) = match &req.params {
        ToolParams::QueryFile {
            tag,
            owner,
            keyword,
            use_index,
        } => (tag.clone(), owner.clone(), keyword.clone(), *use_index),
        _ => {
            return Ok(DispatchResult {
                status: ToolStatus::BadParams,
                body: Vec::new(),
            });
        }
    };

    let store = FILE_ATTR_STORE.exclusive_access();
    let names: Vec<alloc::string::String> = if use_index {
        store.query_indexed(tag.as_deref(), owner.as_deref(), keyword.as_deref())
    } else {
        store.query_full_scan(tag.as_deref(), owner.as_deref(), keyword.as_deref())
    };

    let mut items: Vec<FileInfo> = Vec::with_capacity(names.len());
    for n in names.iter() {
        let attrs = store.get(n);
        let (tags, owner_v, digest_preview) = match attrs {
            Some(a) => {
                let preview_len = a.digest.len().min(48);
                let preview =
                    alloc::string::String::from_utf8_lossy(&a.digest[..preview_len]).to_string();
                (
                    a.tags.clone(),
                    a.kv.get("owner").cloned(),
                    preview,
                )
            }
            None => (Vec::new(), None, alloc::string::String::new()),
        };
        items.push(FileInfo {
            inode_id: 0, // store 不感知 inode_id；演示用 0
            name: n.clone(),
            size: 0,
            tags,
            owner: owner_v,
            digest_preview,
        });
    }

    let body = postcard::to_allocvec(&QueryResult { items })
        .map_err(|_| crate::agent::error::AgentError::InternalError)?;
    Ok(DispatchResult {
        status: ToolStatus::Ok,
        body,
    })
}

// 防止未使用警告
#[allow(dead_code)]
const _UNUSED: fn() -> &'static str = || "dummy";
const _: fn(&str) -> alloc::string::String = ToString::to_string;
