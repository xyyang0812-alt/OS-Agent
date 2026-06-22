//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

/// A simple FIFO scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    ///Add a task to `TaskManager`
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    ///Remove the highest-priority ready task and return it, or `None` if empty.
    ///
    /// 优先级调度（任务五 · 多 Agent 协调）：在就绪队列中选取 `priority` 最大的
    /// 任务；优先级相同的退化为 FIFO（取队列中最靠前者），保证同级公平、不饿死。
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        if self.ready_queue.is_empty() {
            return None;
        }
        let mut best_idx = 0usize;
        let mut best_prio = self.ready_queue[0].priority();
        for i in 1..self.ready_queue.len() {
            let p = self.ready_queue[i].priority();
            // 严格大于才替换 → 同优先级保持先入先出
            if p > best_prio {
                best_prio = p;
                best_idx = i;
            }
        }
        self.ready_queue.remove(best_idx)
    }
}

lazy_static! {
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}
///Interface offered to add task
pub fn add_task(task: Arc<TaskControlBlock>) {
    TASK_MANAGER.exclusive_access().add(task);
}
///Interface offered to pop the first task
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    TASK_MANAGER.exclusive_access().fetch()
}
