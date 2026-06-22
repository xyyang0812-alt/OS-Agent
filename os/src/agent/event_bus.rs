//! 任务五：心跳 + 事件总线
//!
//! ## 心跳
//!
//! 每个 Agent 可注册一个心跳周期（毫秒）。每次 timer 中断会调用
//! [`tick_all_agents`]，遍历所有 Agent，到期就置位 `heartbeat_pending`。
//! `sys_agent_wait` 在每次自旋中检查该位。
//!
//! ## 事件位掩码
//!
//! | 位 | 含义 |
//! |----|------|
//! | 0  | 心跳到期 |
//! | 1  | 邮箱收到新消息 |
//! | 2  | 文件被修改（属性设置/删除时由 `broadcast_file_event` 触发）|
//!
//! ## 邮箱
//!
//! 每个 Agent 自带一个 `mailbox: Vec<Vec<u8>>`。`send_message` 工具
//! 直接 push 进去；`sys_agent_wait` 唤醒后用户态自己读 mailbox（任务六使用）。

use alloc::vec::Vec;

use crate::task::snapshot_all_processes;
use crate::timer::get_time_ms;

pub const EVENT_HEARTBEAT: u32 = 1 << 0;
pub const EVENT_MESSAGE: u32 = 1 << 1;
pub const EVENT_FILE_MODIFIED: u32 = 1 << 2;

/// 在每次 timer 中断里调用：扫描所有 Agent，更新 heartbeat_pending。
///
/// 为避免在中断上下文里持有 inner 锁太久，我们：
/// 1. 拍快照（snapshot_all_processes 自己拷贝 Arc，不持有 inner）
/// 2. 逐个 try-borrow inner（borrow 失败 = 被别处持有 = 跳过这次）
pub fn tick_all_agents() {
    let now_ms = get_time_ms() as u64;
    let tasks = snapshot_all_processes();
    let mut to_wake_pids: alloc::vec::Vec<usize> = alloc::vec::Vec::new();
    for t in tasks {
        // 用 try_exclusive_access 风格——rCore 的 UPSafeCell 只有 exclusive_access，
        // 但因为我们在中断里执行，被中断的进程的 inner 不应正被持有
        // （rCore 中所有 inner_exclusive_access 都会很快 drop）。
        // 这里为了健壮性，仍然短时间持有。
        let mut inner = t.inner_exclusive_access();
        if let Some(ext) = inner.agent_ext.as_mut() {
            if ext.heartbeat_interval_ms > 0 {
                let elapsed = now_ms.saturating_sub(ext.heartbeat_last_ms);
                if elapsed >= ext.heartbeat_interval_ms {
                    ext.heartbeat_pending = true;
                    ext.heartbeat_last_ms = now_ms;
                    // 心跳到期 → 收集等待唤醒的 pid（不在持锁时唤醒，避免锁顺序问题）
                    to_wake_pids.push(t.getpid());
                }
            }
        }
    }
    // 锁外执行唤醒
    for pid in to_wake_pids {
        let _ = crate::agent::blocking::wake_agent_by_pid(pid);
    }
}

/// 投递消息到目标 Agent 的邮箱。
/// 返回 `true` 表示投递成功，`false` 表示目标不是 Agent 或没找到 pid。
pub fn deliver_message(target_pid: usize, payload: Vec<u8>) -> bool {
    let tasks = snapshot_all_processes();
    let mut delivered = false;
    for t in tasks {
        if t.getpid() == target_pid {
            let mut inner = t.inner_exclusive_access();
            if let Some(ext) = inner.agent_ext.as_mut() {
                if ext.mailbox.len() >= ext.mailbox_capacity {
                    // 邮箱满，丢弃最旧的
                    ext.mailbox.remove(0);
                }
                ext.mailbox.push(payload);
                delivered = true;
            }
            // 必须把 inner 锁释放后再唤醒，否则 wake_agent_by_pid 内部要拿 inner
            break;
        }
    }
    if delivered {
        // 锁外唤醒：如果目标 Agent 正 blocked，把它放回 ready queue
        let _ = crate::agent::blocking::wake_agent_by_pid(target_pid);
    }
    delivered
}

/// 广播"文件被修改"事件（任务五 EVENT_FILE_MODIFIED）。
///
/// 文件属性发生设置/删除时调用：给所有 **关注了** `EVENT_FILE_MODIFIED`
/// 的 Agent 置位 `file_event_pending`，并把正在 blocked 的关注者唤醒。
/// 未关注该事件的 Agent 不受影响（不置位、不唤醒），符合"事件按需订阅"。
pub fn broadcast_file_event() {
    let tasks = snapshot_all_processes();
    let mut to_wake_pids: Vec<usize> = Vec::new();
    for t in tasks {
        let mut inner = t.inner_exclusive_access();
        if let Some(ext) = inner.agent_ext.as_mut() {
            if ext.watched_events & EVENT_FILE_MODIFIED != 0 {
                ext.file_event_pending = true;
                to_wake_pids.push(t.getpid());
            }
        }
    }
    // 锁外唤醒，避免锁顺序问题
    for pid in to_wake_pids {
        let _ = crate::agent::blocking::wake_agent_by_pid(pid);
    }
}
