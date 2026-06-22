//! Agent-OS 子系统的 syscall 入口
//!
//! 实现细节都委托给 `crate::agent::*`，这里只做：
//! 1. 用户指针检查 + copy_from_user
//! 2. 错误码翻译
//!
//! 详细规格见 `docs/design/02-syscall-spec.md`。

#![allow(dead_code)]

use alloc::vec;

use agent_proto::{ToolResponse, ToolStatus, decode_request};

use crate::agent::context_area::{
    AgentContextArea, CONTEXT_AREA_DEFAULT_SIZE, read_user_bytes, write_user_bytes,
};
use crate::agent::pcb_ext::{AgentExt, AgentType};
use crate::agent::tool::ToolDispatcher;
use crate::mm::translated_byte_buffer;
use crate::task::current_task;

/// sys_agent_create：把当前进程升级为 Agent 进程
///
/// 参数：`cfg_ptr` 指向用户态的 `AgentCreateCfg`（暂用 0 表示采用默认配置）。
///
/// 返回：成功时返回 0，否则返回 `AgentError::into_isize()`。
pub fn sys_agent_create(_cfg_ptr: usize) -> isize {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();

    if inner.agent_ext.is_some() {
        // 已经是 Agent，幂等返回 0
        return 0;
    }

    // 分配并映射 Agent Context 区
    let area = AgentContextArea::allocate(&mut inner.memory_set, CONTEXT_AREA_DEFAULT_SIZE);

    // 初始化 Header（任务一 MVP 中通过物理页写入）
    area.init_header(&inner.memory_set);

    inner.agent_ext = Some(AgentExt::new(
        AgentType::Normal,
        area.base,
        area.size,
        /* max_nodes */ 128,
        /* max_bytes */ 16 * 1024,
    ));

    0
}

/// sys_agent_info：把当前 Agent 进程的关键信息写到用户提供的缓冲区
///
/// `pid` 暂未使用（后续可支持查别的 Agent，需要权限检查）。
pub fn sys_agent_info(_pid: usize, info_ptr: usize, info_len: usize) -> isize {
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    let ext = match inner.agent_ext.as_ref() {
        Some(e) => e,
        None => return -4,
    };

    // 布局：16 字节小端：
    //   u32 agent_type, u32 context_area_size, u32 path_node_count, u32 loop_state
    let mut buf = [0u8; 16];
    let ty_code: u32 = match ext.agent_type {
        AgentType::Normal => 1,
        AgentType::System => 2,
    };
    let loop_code: u32 = match ext.loop_state {
        crate::agent::pcb_ext::LoopState::Idle => 0,
        crate::agent::pcb_ext::LoopState::Thinking => 1,
        crate::agent::pcb_ext::LoopState::Calling => 2,
        crate::agent::pcb_ext::LoopState::Observing => 3,
        crate::agent::pcb_ext::LoopState::Done => 4,
    };
    buf[0..4].copy_from_slice(&ty_code.to_le_bytes());
    buf[4..8].copy_from_slice(&(ext.context_area_size as u32).to_le_bytes());
    buf[8..12].copy_from_slice(&(ext.path_meta.len() as u32).to_le_bytes());
    buf[12..16].copy_from_slice(&loop_code.to_le_bytes());

    let token = inner.get_user_token();
    drop(inner);

    let chunks = translated_byte_buffer(token, info_ptr as *const u8, info_len.min(buf.len()));
    let mut written = 0usize;
    for chunk in chunks {
        let n = chunk.len().min(buf.len() - written);
        chunk[..n].copy_from_slice(&buf[written..written + n]);
        written += n;
        if written >= buf.len() {
            break;
        }
    }
    written as isize
}

/// sys_tool_call (#510)
///
/// 流程：
/// 1. 检查当前进程是 Agent
/// 2. copy_from_user 读取请求帧
/// 3. `decode_request` 解码（校验 magic / version / postcard body）
/// 4. `ToolDispatcher::dispatch` 执行工具
/// 5. 把结果字节零拷贝写入用户态 Context Area Tool Result Ring
/// 6. 通过 `out_offset_ptr` / `out_len_ptr` 回写定位指针
///
/// 返回值：
/// - 0     成功（仍需通过 `out_len` 判定工具是否 OK）
/// - <0    `AgentError::into_isize()`（参数级错误）
pub fn sys_tool_call(
    req_ptr: usize,
    req_len: usize,
    out_offset_ptr: usize,
    out_len_ptr: usize,
) -> isize {
    if req_ptr == 0 || req_len == 0 || req_len > 64 * 1024 {
        return -1;
    }

    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if inner.agent_ext.is_none() {
        return -4;
    }
    let token = inner.get_user_token();
    let (area_base, area_size) = {
        let ext = inner.agent_ext.as_mut().unwrap().as_mut();
        // 状态机：开始 tool call → Calling
        ext.loop_state = crate::agent::pcb_ext::LoopState::Calling;
        (ext.context_area_base.0, ext.context_area_size)
    };

    // copy_from_user
    let mut req_buf = vec![0u8; req_len];
    read_user_bytes(token, req_ptr, &mut req_buf);
    drop(inner);

    // 解码
    let req = match decode_request(&req_buf) {
        Ok(r) => r,
        Err(_) => return -3,
    };

    // 分发
    let dr = match ToolDispatcher::dispatch(&req) {
        Ok(d) => d,
        Err(e) => return e.into_isize(),
    };

    // 把结果字节写入 Tool Result Ring（仅当成功时）
    let _ = area_size; // demo 简化：当前只覆盖写入 Tool Result Ring 开头
    let (offset, length) = if dr.status == ToolStatus::Ok && !dr.body.is_empty() {
        use crate::agent::context_area::layout;
        if dr.body.len() > layout::TOOL_RESULT_LEN {
            return -5;
        }
        let token = current_task()
            .unwrap()
            .inner_exclusive_access()
            .get_user_token();
        let write_va = area_base + layout::TOOL_RESULT_OFF;
        write_user_bytes(token, write_va, &dr.body);
        (layout::TOOL_RESULT_OFF as u32, dr.body.len() as u32)
    } else {
        (0u32, 0u32)
    };

    // 构造 ToolResponse 帧写回用户提供的输出指针（out_offset/out_len 两个 u32）
    let resp = ToolResponse {
        req_id: req.req_id,
        status: dr.status,
        result_offset: offset,
        result_len: length,
    };

    // 把 offset / len 分别写回两个 u32 指针
    if out_offset_ptr != 0 {
        let token = {
            let task = current_task().unwrap();
            let t = task.inner_exclusive_access().get_user_token();
            t
        };
        write_user_bytes(token, out_offset_ptr, &resp.result_offset.to_le_bytes());
    }
    if out_len_ptr != 0 {
        let token = {
            let task = current_task().unwrap();
            let t = task.inner_exclusive_access().get_user_token();
            t
        };
        write_user_bytes(token, out_len_ptr, &resp.result_len.to_le_bytes());
    }

    // 状态机：tool call 完成 → Observing（Agent 现在该消化结果了）
    {
        let task = current_task().unwrap();
        let mut inner = task.inner_exclusive_access();
        if let Some(ext) = inner.agent_ext.as_mut() {
            ext.loop_state = crate::agent::pcb_ext::LoopState::Observing;
        }
    }

    // syscall 返回值：把 ToolStatus 编码为 isize；Ok=0，其它取负数
    match resp.status {
        ToolStatus::Ok => 0,
        ToolStatus::ToolNotFound => 100,
        ToolStatus::BadParams => 101,
        ToolStatus::PermissionDenied => 102,
        ToolStatus::QuotaExceeded => 103,
        ToolStatus::InternalError => 104,
        ToolStatus::NotImplemented => 105,
    }
}

/// sys_tool_list (#511)
///
/// 把工具列表（postcard 编码的 QueryResult<ToolDescriptor>）拷贝到用户缓冲。
/// 返回写入字节数；缓冲区不足时返回 -2，并不写入。
pub fn sys_tool_list(buf_ptr: usize, buf_len: usize) -> isize {
    if buf_ptr == 0 {
        return -1;
    }
    let body = ToolDispatcher::list();
    if buf_len < body.len() {
        return -2;
    }
    let token = current_task().unwrap().inner_exclusive_access().get_user_token();
    write_user_bytes(token, buf_ptr, &body);
    body.len() as isize
}

/// sys_context_push (#520) — 任务三
///
/// 把 `(req_summary, resp_summary)` 拷贝到内核，调用 `context_path::push_node`
/// 写入用户态 Path Buffer 并维护 PCB 中的元信息。
///
/// 返回：新节点索引（>=0），错误码（<0）。
pub fn sys_context_push(
    req_ptr: usize,
    req_len: usize,
    resp_ptr: usize,
    resp_len: usize,
) -> isize {
    use crate::agent::context_path::push_node;
    if req_len > 4096 || resp_len > 4096 {
        return -2;
    }
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let token = inner.get_user_token();

    // copy_from_user
    let mut req_buf = vec![0u8; req_len];
    if req_len > 0 && req_ptr != 0 {
        read_user_bytes(token, req_ptr, &mut req_buf);
    }
    let mut resp_buf = vec![0u8; resp_len];
    if resp_len > 0 && resp_ptr != 0 {
        read_user_bytes(token, resp_ptr, &mut resp_buf);
    }

    let ext = match inner.agent_ext.as_mut() {
        Some(e) => e.as_mut(),
        None => return -4,
    };
    match push_node(ext, token, &req_buf, &resp_buf) {
        Ok(idx) => idx as isize,
        Err(e) => e.into_isize(),
    }
}

/// sys_context_query (#521) — 任务三
///
/// 把当前路径的元信息（每个节点的 offset + len + seq + ts）序列化为 postcard，
/// 写入 Tool Result Ring，并通过 out 指针返回 `(offset, len)`。
/// 参数 `_start` / `_count` 当前未使用（返回全量元信息）；预留给未来按段查询。
pub fn sys_context_query(
    _start: usize,
    _count: usize,
    out_offset_ptr: usize,
    out_len_ptr: usize,
) -> isize {
    use crate::agent::context_area::layout;
    use crate::agent::context_path::serialize_meta;
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    let token = inner.get_user_token();
    let ext = match inner.agent_ext.as_ref() {
        Some(e) => e,
        None => return -4,
    };
    let body = match serialize_meta(ext) {
        Ok(b) => b,
        Err(e) => return e.into_isize(),
    };
    if body.len() > layout::TOOL_RESULT_LEN {
        return -5;
    }
    let area_base = ext.context_area_base.0;
    drop(inner);

    let write_va = area_base + layout::TOOL_RESULT_OFF;
    write_user_bytes(token, write_va, &body);
    let offset = layout::TOOL_RESULT_OFF as u32;
    let length = body.len() as u32;
    if out_offset_ptr != 0 {
        write_user_bytes(token, out_offset_ptr, &offset.to_le_bytes());
    }
    if out_len_ptr != 0 {
        write_user_bytes(token, out_len_ptr, &length.to_le_bytes());
    }
    0
}

/// sys_context_rollback (#522)
///
/// 保留前 node_count 个节点，丢弃后面的。
pub fn sys_context_rollback(node_count: usize) -> isize {
    use crate::agent::context_path::rollback;
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let token = inner.get_user_token();
    let ext = match inner.agent_ext.as_mut() {
        Some(e) => e.as_mut(),
        None => return -4,
    };
    match rollback(ext, token, node_count) {
        Ok(()) => 0,
        Err(e) => e.into_isize(),
    }
}

/// sys_context_clear (#523)
pub fn sys_context_clear() -> isize {
    use crate::agent::context_path::clear;
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let token = inner.get_user_token();
    let ext = match inner.agent_ext.as_mut() {
        Some(e) => e.as_mut(),
        None => return -4,
    };
    clear(ext, token);
    0
}

/// sys_agent_set_loop_state (#536)
///
/// 让 Agent 显式声明当前 Loop 状态（Idle/Thinking/Calling/Observing/Done）。
/// 内核也会在 sys_tool_call / sys_agent_wait 内部自动推动状态机，
/// 但 Agent 可用这个 syscall 覆盖（典型场景：宣告"任务完成"切到 Done）。
///
/// 编码：0=Idle, 1=Thinking, 2=Calling, 3=Observing, 4=Done
pub fn sys_agent_set_loop_state(state_code: u32) -> isize {
    use crate::agent::pcb_ext::LoopState;
    let new_state = match state_code {
        0 => LoopState::Idle,
        1 => LoopState::Thinking,
        2 => LoopState::Calling,
        3 => LoopState::Observing,
        4 => LoopState::Done,
        _ => return -1,
    };
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let ext = match inner.agent_ext.as_mut() {
        Some(e) => e.as_mut(),
        None => return -4,
    };
    ext.loop_state = new_state;
    0
}

/// sys_agent_set_priority (#539) — 任务五：多 Agent 协调调度
///
/// 设置当前任务的调度优先级（数值越大越优先，钳到 `MAX_PRIORITY`）。
/// 调度器 `TaskManager::fetch` 会在就绪队列中优先取优先级最高者。
///
/// 返回：设置后的优先级值（>=0）。
pub fn sys_agent_set_priority(priority: usize) -> isize {
    let p = priority.min(crate::task::MAX_PRIORITY);
    let task = current_task().unwrap();
    task.inner_exclusive_access().priority = p;
    p as isize
}

/// sys_agent_heartbeat_set (#530)
pub fn sys_agent_heartbeat_set(interval_ms: usize) -> isize {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let ext = match inner.agent_ext.as_mut() {
        Some(e) => e.as_mut(),
        None => return -4,
    };
    ext.heartbeat_interval_ms = interval_ms as u64;
    ext.heartbeat_last_ms = crate::timer::get_time_ms() as u64;
    ext.heartbeat_pending = false;
    0
}

/// sys_agent_heartbeat_stop (#531)
pub fn sys_agent_heartbeat_stop() -> isize {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let ext = match inner.agent_ext.as_mut() {
        Some(e) => e.as_mut(),
        None => return -4,
    };
    ext.heartbeat_interval_ms = 0;
    ext.heartbeat_pending = false;
    0
}

/// sys_agent_watch (#532) —— 简化版：只关注事件位掩码
pub fn sys_agent_watch(event_mask: u32, _filter_ptr: usize, _filter_len: usize) -> isize {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let ext = match inner.agent_ext.as_mut() {
        Some(e) => e.as_mut(),
        None => return -4,
    };
    ext.watched_events |= event_mask;
    0
}

/// sys_agent_unwatch (#534)
pub fn sys_agent_unwatch(event_mask: usize) -> isize {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let ext = match inner.agent_ext.as_mut() {
        Some(e) => e.as_mut(),
        None => return -4,
    };
    ext.watched_events &= !(event_mask as u32);
    0
}

/// sys_agent_wait (#533) ——  **真休眠**：进程在等待期间从 ready queue 移除
///
/// 阻塞当前 Agent，直到：
/// - 心跳到期（pending=true）→ tick_all_agents 唤醒
/// - 邮箱收到至少 1 条消息 → deliver_message 唤醒
/// - 超过 `timeout_ms`（<0 表示永久等）→ tick_wake_timeouts 唤醒
///
/// **事件过滤**：只有在 `watched_events` 位掩码里设置过的事件才会让 wait
/// 返回；用户从未调过 `sys_agent_watch` 时 `watched_events == 0`，作为
/// 默认我们当作"关心全部事件"——避免新手用户漏掉 watch 后陷入永久等待。
///
/// 返回值：触发原因位掩码（见 `event_bus::EVENT_*`），0 表示超时无事件。
///
/// 状态机：进入时把 `loop_state` 切到 `Thinking`，返回前切回 `Idle`。
///
/// 实现细节：使用 `agent::blocking::block_current_agent`——把 task 真正
/// 移出 ready queue 直到外部 wake。期间 processor 永远 fetch 不到这个
/// task，**它的时间片为 0**，对应要求中"无事件时不消耗 CPU"。
pub fn sys_agent_wait(timeout_ms: i64) -> isize {
    use crate::agent::blocking::block_current_agent;
    use crate::agent::event_bus::{EVENT_FILE_MODIFIED, EVENT_HEARTBEAT, EVENT_MESSAGE};
    use crate::agent::pcb_ext::LoopState;
    use crate::timer::get_time_ms;

    let start_ms = get_time_ms() as i64;

    // 状态机：进入 wait → Thinking
    {
        let task = current_task().unwrap();
        let mut inner = task.inner_exclusive_access();
        if let Some(ext) = inner.agent_ext.as_mut() {
            ext.loop_state = LoopState::Thinking;
        }
    }

    let absolute_deadline_ms: Option<u64> = if timeout_ms < 0 {
        None
    } else {
        Some((get_time_ms() as i64 + timeout_ms).max(0) as u64)
    };

    loop {
        // 抓一帧 pending 状态（最小持锁）
        let cause = {
            let task = current_task().unwrap();
            let mut inner = task.inner_exclusive_access();
            let ext = match inner.agent_ext.as_mut() {
                Some(e) => e.as_mut(),
                None => return -4,
            };
            let mut c: u32 = 0;
            if ext.heartbeat_pending {
                c |= EVENT_HEARTBEAT;
            }
            if !ext.mailbox.is_empty() {
                c |= EVENT_MESSAGE;
            }
            if ext.file_event_pending {
                c |= EVENT_FILE_MODIFIED;
            }
            // 应用 watched_events 过滤；为 0 时视为"关心全部"
            let m = if ext.watched_events == 0 {
                EVENT_HEARTBEAT | EVENT_MESSAGE
            } else {
                ext.watched_events
            };
            let triggered = c & m;
            // 只消费命中且被关心的事件
            if triggered & EVENT_HEARTBEAT != 0 {
                ext.heartbeat_pending = false;
            }
            if triggered & EVENT_FILE_MODIFIED != 0 {
                ext.file_event_pending = false;
            }
            triggered
        };

        if cause != 0 {
            // 状态机：唤醒 → Idle
            let task = current_task().unwrap();
            let mut inner = task.inner_exclusive_access();
            if let Some(ext) = inner.agent_ext.as_mut() {
                ext.loop_state = LoopState::Idle;
            }
            return cause as isize;
        }

        // 超时检查
        if timeout_ms >= 0 {
            let now = get_time_ms() as i64;
            if now - start_ms >= timeout_ms {
                let task = current_task().unwrap();
                let mut inner = task.inner_exclusive_access();
                if let Some(ext) = inner.agent_ext.as_mut() {
                    ext.loop_state = LoopState::Idle;
                }
                return 0;
            }
        }

        // **真休眠**：从 ready queue 移除，等待 wake_agent_by_pid 或 tick_wake_timeouts
        // 唤醒后 schedule 切回这里，再回到 loop 开头重抓 cause
        block_current_agent(absolute_deadline_ms);
    }
}

/// sys_file_attr_set (#538) — 任务四：给文件设置一个标签
///
/// 参数：`name_ptr/name_len` 文件名，`tag_ptr/tag_len` 标签。
/// 调用方无需是 Agent（属性表是全局的），但要求字符串合法 UTF-8。
///
/// 返回：0 成功；-1 参数非法；-7 文件名/标签非 UTF-8。
pub fn sys_file_attr_set(
    name_ptr: usize,
    name_len: usize,
    tag_ptr: usize,
    tag_len: usize,
) -> isize {
    if name_ptr == 0 || name_len == 0 || name_len > 256 || tag_len > 256 {
        return -1;
    }
    let token = current_task()
        .unwrap()
        .inner_exclusive_access()
        .get_user_token();

    let mut name_buf = vec![0u8; name_len];
    read_user_bytes(token, name_ptr, &mut name_buf);
    let mut tag_buf = vec![0u8; tag_len];
    if tag_len > 0 && tag_ptr != 0 {
        read_user_bytes(token, tag_ptr, &mut tag_buf);
    }

    let name = match core::str::from_utf8(&name_buf) {
        Ok(s) => s,
        Err(_) => return -7,
    };
    let tag = match core::str::from_utf8(&tag_buf) {
        Ok(s) => s,
        Err(_) => return -7,
    };

    crate::agent::file_attr::FILE_ATTR_STORE
        .exclusive_access()
        .add_tag(name, tag);
    // 属性变更 → 广播文件事件，唤醒关注 EVENT_FILE_MODIFIED 的 Agent
    crate::agent::event_bus::broadcast_file_event();
    0
}

/// sys_file_attr_del (#537) — 任务四：删除文件属性
///
/// 参数：`name_ptr/name_len` 文件名，`tag_ptr/tag_len` 标签。
/// - `tag_len > 0`：删除该文件的指定标签
/// - `tag_len == 0`：删除该文件的全部属性
///
/// 返回：1 删掉了已存在的项；0 目标本不存在；-1 参数非法；-7 非 UTF-8。
pub fn sys_file_attr_del(
    name_ptr: usize,
    name_len: usize,
    tag_ptr: usize,
    tag_len: usize,
) -> isize {
    if name_ptr == 0 || name_len == 0 || name_len > 256 || tag_len > 256 {
        return -1;
    }
    let token = current_task()
        .unwrap()
        .inner_exclusive_access()
        .get_user_token();

    let mut name_buf = vec![0u8; name_len];
    read_user_bytes(token, name_ptr, &mut name_buf);
    let name = match core::str::from_utf8(&name_buf) {
        Ok(s) => s,
        Err(_) => return -7,
    };

    let changed = {
        let mut store = crate::agent::file_attr::FILE_ATTR_STORE.exclusive_access();
        if tag_len == 0 {
            store.remove_file(name)
        } else {
            let mut tag_buf = vec![0u8; tag_len];
            read_user_bytes(token, tag_ptr, &mut tag_buf);
            let tag = match core::str::from_utf8(&tag_buf) {
                Ok(s) => s,
                Err(_) => return -7,
            };
            store.remove_tag(name, tag)
        }
    };
    if changed {
        // 真正删掉了东西才广播文件事件
        crate::agent::event_bus::broadcast_file_event();
        1
    } else {
        0
    }
}

/// sys_mailbox_recv (#535)  — 任务五附加：取走邮箱里的一条消息
///
/// 与 `read_user_bytes` 写出协议帧不同，邮箱消息直接当作 raw bytes 写入用户缓冲。
/// 返回值：写入字节数 (>=0)，或 `-6` 邮箱空。
pub fn sys_mailbox_recv(buf_ptr: usize, buf_len: usize) -> isize {
    if buf_ptr == 0 {
        return -1;
    }
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let token = inner.get_user_token();
    let ext = match inner.agent_ext.as_mut() {
        Some(e) => e.as_mut(),
        None => return -4,
    };
    if ext.mailbox.is_empty() {
        return -6;
    }
    let msg = ext.mailbox.remove(0);
    let n = buf_len.min(msg.len());
    drop(inner);
    write_user_bytes(token, buf_ptr, &msg[..n]);
    n as isize
}
