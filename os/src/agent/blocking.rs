//! 任务五：Agent 真休眠（block / wake）
//!
//! ## 设计动机
//!
//! 要求文档明确说："Agent 在无事件时正确休眠，不消耗 CPU"。基线 rCore-ch6
//! 只有 Ready/Running/Zombie 三种状态，`sys_agent_wait` 之前用 yield-loop
//! 模拟阻塞，CPU 仍会被该 Agent 占用调度时间片。
//!
//! 本模块实现"真休眠"：
//! 1. 把 `TaskStatus` 扩出 `Blocked` 变体（在 `task.rs`）
//! 2. 维护全局 `BLOCKED_AGENTS: Vec<BlockedEntry>`——被挂起的 Agent 不
//!    在 ready queue 里，processor `fetch_task` 永远拿不到它们
//! 3. **事件触发唤醒**：心跳到期、消息投递、超时——都把 Agent 移回 ready queue
//!
//! ## 关键性质
//!
//! - 只有 Agent 进程可被挂起，普通进程行为不变（不影响 rCore 既有调度语义）
//! - `BLOCKED_AGENTS` 持有 `Arc<TCB>`，确保 task 在挂起期间不会被 drop
//! - 所有"放回 ready queue"的路径都集中在 `wake_one`，便于审计

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::sync::UPSafeCell;
use crate::task::{
    TaskContext, TaskControlBlock, TaskStatus, add_task, schedule, take_current_task,
};

/// 一条 blocked 记录
pub struct BlockedEntry {
    pub task: Arc<TaskControlBlock>,
    /// 绝对唤醒时刻（毫秒）；None 表示"无限等"
    pub wake_deadline_ms: Option<u64>,
}

lazy_static::lazy_static! {
    /// 所有当前被挂起的 Agent 任务
    pub static ref BLOCKED_AGENTS: UPSafeCell<Vec<BlockedEntry>> =
        unsafe { UPSafeCell::new(Vec::new()) };
}

/// 把当前正在运行的 Agent 任务挂起。
///
/// - `wake_deadline_ms = None` ⇒ 永久等（直到外部 wake）
/// - `wake_deadline_ms = Some(t)` ⇒ 不晚于 `t` 毫秒被唤醒（由 timer 中断扫描）
///
/// 调用约定：
/// - 进入前调用方必须**已经把 `loop_state` / `heartbeat_pending` 等状态摆好**，
///   因为本函数不再读用户参数
/// - 进入前调用方必须**已经 drop 掉所有 inner 锁**
///
/// 函数从 `schedule` 切走后会一直挂起，直到事件唤醒它回来。
pub fn block_current_agent(wake_deadline_ms: Option<u64>) {
    // 拿走 processor.current
    let task = take_current_task().expect("block_current_agent: no current task");

    // 切到 Blocked 状态，并取出 task_cx_ptr 供 schedule 使用
    let task_cx_ptr: *mut TaskContext = {
        let mut inner = task.inner_exclusive_access();
        inner.task_status = TaskStatus::Blocked;
        &mut inner.task_cx as *mut TaskContext
    };

    // 入阻塞列表（保留 Arc，避免 task 被 drop）
    BLOCKED_AGENTS.exclusive_access().push(BlockedEntry {
        task,
        wake_deadline_ms,
    });

    // 切走——processor 回到 idle 协程；下次该 task 被 wake 后才会被 fetch_task 取出
    schedule(task_cx_ptr);
}

/// 按 pid 把一个 blocked agent 移回 ready queue。
///
/// 返回 true 表示成功唤醒；false 表示该 pid 当前不在阻塞列表
/// （可能它本来就是 Ready/Running，或者根本不是 Agent）。
pub fn wake_agent_by_pid(pid: usize) -> bool {
    let entry = {
        let mut blocked = BLOCKED_AGENTS.exclusive_access();
        match blocked.iter().position(|e| e.task.getpid() == pid) {
            Some(idx) => Some(blocked.swap_remove(idx)),
            None => None,
        }
    };
    match entry {
        Some(e) => {
            wake_one(e.task);
            true
        }
        None => false,
    }
}

/// 在每次 timer tick 调用：把所有"deadline 已过"的 agent 唤醒。
pub fn tick_wake_timeouts(now_ms: u64) {
    let mut to_wake: Vec<Arc<TaskControlBlock>> = Vec::new();
    {
        let mut blocked = BLOCKED_AGENTS.exclusive_access();
        let mut i = 0;
        while i < blocked.len() {
            let should_wake = match blocked[i].wake_deadline_ms {
                Some(d) => now_ms >= d,
                None => false,
            };
            if should_wake {
                let e = blocked.swap_remove(i);
                to_wake.push(e.task);
            } else {
                i += 1;
            }
        }
    }
    for t in to_wake {
        wake_one(t);
    }
}

/// 内部：把任务真正放回 ready queue
fn wake_one(task: Arc<TaskControlBlock>) {
    task.inner_exclusive_access().task_status = TaskStatus::Ready;
    add_task(task);
}

/// 当前 BLOCKED 列表长度（调试/统计用）
#[allow(dead_code)]
pub fn blocked_count() -> usize {
    BLOCKED_AGENTS.exclusive_access().len()
}
