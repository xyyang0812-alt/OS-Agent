//! Agent Loop 上下文路径（任务三）
//!
//! 元信息（每个节点的 offset / len / 时戳 / 序号）保存在内核 PCB 的
//! [`AgentExt::path_meta`]，数据本体在用户态 Context Area Path Buffer 段。
//!
//! ## 存储格式（Path Buffer 中每个节点的二进制布局）
//!
//! ```text
//! ┌──────────┬──────────┬────────────┬────────────┬──────────────┬──────────────┐
//! │ seq u64  │ ts  u64  │ req_len u32│ resp_len u32│ req_bytes   │ resp_bytes  │
//! └──────────┴──────────┴────────────┴────────────┴──────────────┴──────────────┘
//! ```
//!
//! 设计选择：
//! - **顺序追加 + 整体移位淘汰**：实现简单，对教学量级（<= 几百个节点）足够
//! - **元信息持有 offset**：用户态查询时直接拿 offset 读取，无需重新扫描
//! - **同时维护 Context Area Header 的 path_used_bytes/path_node_count**：
//!   方便用户态零拷贝读路径概况

use alloc::vec;
use alloc::vec::Vec;

use crate::agent::context_area::{layout, read_user_bytes, write_user_bytes};
use crate::agent::error::{AgentError, AgentResult};
use crate::agent::pcb_ext::{AgentExt, EvictPolicy, PathNodeMeta};
use crate::timer::get_time;

/// Path 节点二进制头部长度（seq + ts + req_len + resp_len）
pub const NODE_HEADER_SIZE: usize = 8 + 8 + 4 + 4;

/// 估算单个节点占用 Path Buffer 的字节数
pub fn node_total_size(req_len: usize, resp_len: usize) -> usize {
    NODE_HEADER_SIZE + req_len + resp_len
}

/// 把内核 PCB 维护的 `path_used_bytes` 和 `path_node_count` 同步到用户态 Header。
fn sync_header(ext: &AgentExt, token: usize) {
    let used = ext.path_used_bytes;
    let count = ext.path_meta.len() as u32;
    // Header 中 path_used_bytes 偏移 = 4(magic)+4(version)+8(seq)+12(tool_result_*)+8(path_buffer_off+len)
    //   = 36 字节 → 见 context_area::AreaHeader 字段顺序
    // path_node_count 紧跟其后 = 40
    let base = ext.context_area_base.0;
    write_user_bytes(token, base + 36, &used.to_le_bytes());
    write_user_bytes(token, base + 40, &count.to_le_bytes());
}

/// 把一个完整节点写入 Path Buffer 指定偏移。
fn write_node_at(
    token: usize,
    path_buffer_va: usize,
    offset_in_buffer: u32,
    seq: u64,
    ts: u64,
    req: &[u8],
    resp: &[u8],
) {
    let mut header = [0u8; NODE_HEADER_SIZE];
    header[0..8].copy_from_slice(&seq.to_le_bytes());
    header[8..16].copy_from_slice(&ts.to_le_bytes());
    header[16..20].copy_from_slice(&(req.len() as u32).to_le_bytes());
    header[20..24].copy_from_slice(&(resp.len() as u32).to_le_bytes());
    let abs = path_buffer_va + offset_in_buffer as usize;
    write_user_bytes(token, abs, &header);
    write_user_bytes(token, abs + NODE_HEADER_SIZE, req);
    write_user_bytes(token, abs + NODE_HEADER_SIZE + req.len(), resp);
}

/// 把 Path Buffer 中 [from, from+len) 的字节向前移动 `shift` 字节
/// （用于淘汰最早节点后整体压缩）。
fn shift_path_buffer_left(token: usize, path_buffer_va: usize, from: u32, len: u32, shift: u32) {
    if len == 0 || shift == 0 {
        return;
    }
    // 简单实现：分块读 → 写回左移后的位置
    const CHUNK: usize = 256;
    let mut buf = [0u8; CHUNK];
    let mut copied: u32 = 0;
    while copied < len {
        let n = ((len - copied) as usize).min(CHUNK);
        read_user_bytes(token, path_buffer_va + (from + copied) as usize, &mut buf[..n]);
        write_user_bytes(
            token,
            path_buffer_va + (from + copied - shift) as usize,
            &buf[..n],
        );
        copied += n as u32;
    }
}

/// 选取一个待淘汰节点的索引（按当前配置的策略）
fn pick_victim(ext: &AgentExt) -> Option<usize> {
    if ext.path_meta.is_empty() {
        return None;
    }
    match ext.path_quota.policy {
        EvictPolicy::Fifo => Some(0),
        EvictPolicy::Lru => ext
            .path_meta
            .iter()
            .enumerate()
            .min_by_key(|(_, m): &(usize, &PathNodeMeta)| m.write_time)
            .map(|(i, _)| i),
    }
}

/// 淘汰一个节点：从 meta 中移除 + 把 Path Buffer 中后续节点整体前移
fn evict_one(ext: &mut AgentExt, token: usize) -> AgentResult<()> {
    let victim_idx = pick_victim(ext).ok_or(AgentError::NotFound)?;
    let victim = ext.path_meta[victim_idx];
    let path_buffer_va = ext.context_area_base.0 + layout::PATH_BUFFER_OFF;

    // 1. 把 [victim.end, used) 整体前移 victim.len
    let move_from = victim.offset_in_buffer + victim.len;
    let move_len = ext.path_used_bytes.saturating_sub(move_from);
    shift_path_buffer_left(token, path_buffer_va, move_from, move_len, victim.len);

    // 2. 更新 meta：删除 victim_idx；对其后所有节点的 offset 减去 victim.len
    ext.path_meta.remove(victim_idx);
    for m in ext.path_meta.iter_mut() {
        if m.offset_in_buffer >= move_from {
            m.offset_in_buffer -= victim.len;
        }
    }

    // 3. 更新统计
    ext.path_used_bytes = ext.path_used_bytes.saturating_sub(victim.len);
    Ok(())
}

/// 追加一个节点到路径末尾。返回新节点在 path_meta 中的索引。
///
/// - 若违反 max_bytes / max_nodes 配额，循环淘汰直到能容纳；
///   若单个节点本身就超过整个 Path Buffer，返回 `QuotaExceeded`。
pub fn push_node(
    ext: &mut AgentExt,
    token: usize,
    req_summary: &[u8],
    resp_summary: &[u8],
) -> AgentResult<usize> {
    let size = node_total_size(req_summary.len(), resp_summary.len()) as u32;

    // 单节点超过 Path Buffer 整段
    if size as usize > layout::PATH_BUFFER_LEN {
        return Err(AgentError::QuotaExceeded);
    }
    // 单节点超过用户配置的 max_bytes：也不能放下
    if size > ext.path_quota.max_bytes {
        return Err(AgentError::QuotaExceeded);
    }

    // 淘汰至有空位
    while ext.path_used_bytes + size > ext.path_quota.max_bytes
        || ext.path_meta.len() as u32 >= ext.path_quota.max_nodes
    {
        if ext.path_meta.is_empty() {
            // 不应该发生
            return Err(AgentError::InternalError);
        }
        evict_one(ext, token)?;
    }

    let offset = ext.path_used_bytes;
    let seq = ext.next_seq;
    let now = get_time() as u64;
    let path_buffer_va = ext.context_area_base.0 + layout::PATH_BUFFER_OFF;

    write_node_at(
        token,
        path_buffer_va,
        offset,
        seq,
        now,
        req_summary,
        resp_summary,
    );

    let meta = PathNodeMeta {
        offset_in_buffer: offset,
        len: size,
        write_time: now,
        seq,
    };
    ext.path_meta.push(meta);
    ext.path_used_bytes += size;
    ext.next_seq += 1;

    sync_header(ext, token);
    Ok(ext.path_meta.len() - 1)
}

/// 回溯：保留前 `node_count` 个节点，丢弃后面的（不需要移动 Path Buffer，
/// 只截断 meta 并减少 path_used_bytes 即可——尾部数据废弃但下次 push 会覆盖）。
pub fn rollback(ext: &mut AgentExt, token: usize, node_count: usize) -> AgentResult<()> {
    if node_count > ext.path_meta.len() {
        return Err(AgentError::NotFound);
    }
    if node_count == 0 {
        ext.path_meta.clear();
        ext.path_used_bytes = 0;
    } else {
        let last = ext.path_meta[node_count - 1];
        let new_used = last.offset_in_buffer + last.len;
        ext.path_meta.truncate(node_count);
        ext.path_used_bytes = new_used;
    }
    sync_header(ext, token);
    Ok(())
}

/// 清空路径
pub fn clear(ext: &mut AgentExt, token: usize) {
    ext.path_meta.clear();
    ext.path_used_bytes = 0;
    sync_header(ext, token);
}

/// 把 path_meta 序列化为 Vec<u8>（postcard），供 sys_context_query 写入 Tool Result Ring
pub fn serialize_meta(ext: &AgentExt) -> AgentResult<Vec<u8>> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct MetaSerde {
        offset: u32,
        len: u32,
        seq: u64,
        write_time: u64,
    }
    #[derive(Serialize)]
    struct MetaList {
        items: alloc::vec::Vec<MetaSerde>,
    }

    let items: alloc::vec::Vec<MetaSerde> = ext
        .path_meta
        .iter()
        .map(|m| MetaSerde {
            offset: m.offset_in_buffer,
            len: m.len,
            seq: m.seq,
            write_time: m.write_time,
        })
        .collect();
    let _ = vec![0u8; 0]; // silence unused alloc::vec import warning
    postcard::to_allocvec(&MetaList { items }).map_err(|_| AgentError::InternalError)
}
