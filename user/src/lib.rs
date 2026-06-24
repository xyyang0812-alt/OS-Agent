#![no_std]
#[macro_use]
pub mod console;
mod lang_items;
mod syscall;

extern crate alloc;
#[macro_use]
extern crate bitflags;

use buddy_system_allocator::LockedHeap;
use core::ptr::addr_of_mut;
use syscall::*;

unsafe extern "Rust" {
    fn main() -> i32;
}

const USER_HEAP_SIZE: usize = 32768;

static mut HEAP_SPACE: [u8; USER_HEAP_SIZE] = [0; USER_HEAP_SIZE];

#[global_allocator]
static HEAP: LockedHeap = LockedHeap::empty();

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    unsafe {
        HEAP.lock()
            .init(addr_of_mut!(HEAP_SPACE) as usize, USER_HEAP_SIZE);
    }
    unsafe {
        exit(main());
    }
}

bitflags! {
    pub struct OpenFlags: u32 {
        const RDONLY = 0;
        const WRONLY = 1 << 0;
        const RDWR = 1 << 1;
        const CREATE = 1 << 9;
        const TRUNC = 1 << 10;
    }
}

pub fn open(path: &str, flags: OpenFlags) -> isize {
    sys_open(path, flags.bits)
}
pub fn close(fd: usize) -> isize {
    sys_close(fd)
}
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    sys_read(fd, buf)
}
pub fn write(fd: usize, buf: &[u8]) -> isize {
    sys_write(fd, buf)
}
pub fn exit(exit_code: i32) -> ! {
    sys_exit(exit_code);
}
pub fn yield_() -> isize {
    sys_yield()
}
pub fn get_time() -> isize {
    sys_get_time()
}
pub fn getpid() -> isize {
    sys_getpid()
}
pub fn fork() -> isize {
    sys_fork()
}
pub fn exec(path: &str) -> isize {
    sys_exec(path)
}
pub fn wait(exit_code: &mut i32) -> isize {
    loop {
        match sys_waitpid(-1, exit_code as *mut _) {
            -2 => {
                yield_();
            }
            // -1 or a real pid
            exit_pid => return exit_pid,
        }
    }
}

pub fn waitpid(pid: usize, exit_code: &mut i32) -> isize {
    loop {
        match sys_waitpid(pid as isize, exit_code as *mut _) {
            -2 => {
                yield_();
            }
            // -1 or a real pid
            exit_pid => return exit_pid,
        }
    }
}
pub fn sleep(period_ms: usize) {
    let start = sys_get_time();
    while sys_get_time() < start + period_ms as isize {
        sys_yield();
    }
}

// ============ Agent-OS User API ============

/// Agent Context 区在用户地址空间中的固定基址，
/// 必须与内核 `os/src/agent/context_area.rs::CONTEXT_AREA_BASE` 保持一致。
pub const AGENT_CONTEXT_BASE: usize = 0x8000_0000;

/// 把当前进程升级为 Agent 进程，分配 Context 区。
///
/// 返回 0 表示成功；其它值见 `docs/design/02-syscall-spec.md` 错误码表。
pub fn agent_create() -> isize {
    sys_agent_create(0)
}

/// 把当前 Agent 元信息读到栈上结构体。
///
/// 布局：16 字节小端：
/// u32 agent_type, u32 context_area_size, u32 path_node_count, u32 loop_state
///
/// `loop_state` 编码：0=Idle, 1=Thinking, 2=Calling, 3=Observing, 4=Done
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
pub struct AgentInfo {
    pub agent_type: u32,
    pub context_area_size: u32,
    pub path_node_count: u32,
    pub loop_state: u32,
}

/// Agent Loop 状态码（与内核 `LoopState` enum 对齐）
pub const LOOP_STATE_IDLE: u32 = 0;
pub const LOOP_STATE_THINKING: u32 = 1;
pub const LOOP_STATE_CALLING: u32 = 2;
pub const LOOP_STATE_OBSERVING: u32 = 3;
pub const LOOP_STATE_DONE: u32 = 4;

pub fn loop_state_name(s: u32) -> &'static str {
    match s {
        0 => "Idle",
        1 => "Thinking",
        2 => "Calling",
        3 => "Observing",
        4 => "Done",
        _ => "?",
    }
}

pub fn agent_info() -> Result<AgentInfo, isize> {
    let mut info = AgentInfo::default();
    let r = sys_agent_info(
        0,
        &mut info as *mut AgentInfo as *mut u8,
        core::mem::size_of::<AgentInfo>(),
    );
    if r < 0 {
        Err(r)
    } else {
        Ok(info)
    }
}

/// 把 Agent Context 区当成一段只读字节切片（用户态零拷贝读取）。
///
/// # Safety
/// 调用者必须先调用 `agent_create()` 让内核完成映射，
/// 否则访问该地址会触发 page fault。
pub unsafe fn agent_context_area(len: usize) -> &'static [u8] {
    unsafe { core::slice::from_raw_parts(AGENT_CONTEXT_BASE as *const u8, len) }
}

// ============ Tool Call 高层 API ============

pub use agent_proto;

/// 工具调用返回值
#[derive(Debug, Clone)]
pub struct ToolCallOutcome {
    pub status_code: isize,
    pub result_offset: u32,
    pub result_len: u32,
}

impl ToolCallOutcome {
    pub fn is_ok(&self) -> bool {
        self.status_code == 0
    }
    /// 直接从 Agent Context 区中读取结果字节（无 syscall，零拷贝）
    pub fn result_bytes(&self) -> &'static [u8] {
        if self.result_len == 0 {
            return &[];
        }
        let ptr = (AGENT_CONTEXT_BASE + self.result_offset as usize) as *const u8;
        unsafe { core::slice::from_raw_parts(ptr, self.result_len as usize) }
    }
}

/// 发送一个 Tool Request，返回工具调用结果（含状态码和结果定位指针）。
///
/// 内部流程：编码请求 → ecall → 内核分发 → 把结果字节写入用户态 Context 区
/// → 我们用 result_offset/len 直接读 Context 区，零拷贝。
pub fn tool_call(req: &agent_proto::ToolRequest) -> Result<ToolCallOutcome, isize> {
    // 序列化请求帧到栈缓冲
    let mut buf = [0u8; 1024];
    let n = agent_proto::encode_request(req, &mut buf).map_err(|_| -100isize)?;
    let mut out_offset: u32 = 0;
    let mut out_len: u32 = 0;
    let r = sys_tool_call(buf.as_ptr(), n, &mut out_offset, &mut out_len);
    if r < 0 {
        return Err(r);
    }
    Ok(ToolCallOutcome {
        status_code: r,
        result_offset: out_offset,
        result_len: out_len,
    })
}

/// 拉取工具列表（postcard 编码的 QueryResult<ToolDescriptor>）。
/// 返回字节数；缓冲不足返回 Err(-2)。
pub fn tool_list(buf: &mut [u8]) -> Result<usize, isize> {
    let r = sys_tool_list(buf.as_mut_ptr(), buf.len());
    if r < 0 {
        Err(r)
    } else {
        Ok(r as usize)
    }
}

// ============ Context Path API ============

/// 单个 Path Node 的元信息（与内核 `PathNodeMeta` 同构，用于反序列化 sys_context_query 的结果）
#[derive(Debug, serde::Deserialize)]
pub struct PathNodeMeta {
    pub offset: u32,
    pub len: u32,
    pub seq: u64,
    pub write_time: u64,
}

#[derive(Debug, serde::Deserialize)]
pub struct PathMetaList {
    pub items: alloc::vec::Vec<PathNodeMeta>,
}

/// 向当前路径追加一个节点
pub fn context_push(req: &[u8], resp: &[u8]) -> Result<usize, isize> {
    let r = sys_context_push(req.as_ptr(), req.len(), resp.as_ptr(), resp.len());
    if r < 0 {
        Err(r)
    } else {
        Ok(r as usize)
    }
}

/// 查询当前路径的元信息（写入 Context Area Tool Result Ring，零拷贝读）
pub fn context_query_meta() -> Result<PathMetaList, isize> {
    let mut out_offset: u32 = 0;
    let mut out_len: u32 = 0;
    let r = sys_context_query(0, usize::MAX, &mut out_offset, &mut out_len);
    if r < 0 {
        return Err(r);
    }
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (AGENT_CONTEXT_BASE + out_offset as usize) as *const u8,
            out_len as usize,
        )
    };
    postcard::from_bytes::<PathMetaList>(bytes).map_err(|_| -100)
}

pub fn context_rollback(keep_count: usize) -> isize {
    sys_context_rollback(keep_count)
}

pub fn context_clear() -> isize {
    sys_context_clear()
}

// ============ Agent Loop API ============

pub const EVENT_HEARTBEAT: u32 = 1 << 0;
pub const EVENT_MESSAGE: u32 = 1 << 1;
pub const EVENT_FILE_MODIFIED: u32 = 1 << 2;

pub fn agent_heartbeat_set(interval_ms: usize) -> isize {
    sys_agent_heartbeat_set(interval_ms)
}
pub fn agent_heartbeat_stop() -> isize {
    sys_agent_heartbeat_stop()
}
pub fn agent_watch(event_mask: u32) -> isize {
    sys_agent_watch(event_mask)
}
pub fn agent_unwatch(event_mask: u32) -> isize {
    sys_agent_unwatch(event_mask)
}
/// 阻塞等待，timeout_ms < 0 表示永久。返回触发原因位掩码。
pub fn agent_wait(timeout_ms: i64) -> u32 {
    let r = sys_agent_wait(timeout_ms);
    if r < 0 { 0 } else { r as u32 }
}
/// 从邮箱取一条消息到用户缓冲。返回写入字节数 / -6 邮箱空。
pub fn mailbox_recv(buf: &mut [u8]) -> isize {
    sys_mailbox_recv(buf.as_mut_ptr(), buf.len())
}

/// 主动声明当前 Agent Loop 状态（典型场景：宣告任务完成切到 Done）。
pub fn agent_set_loop_state(state: u32) -> isize {
    sys_agent_set_loop_state(state)
}

// ============ 文件属性 设置 / 删除 API（任务四）============

/// 给文件设置一个标签（属性的"设置"）。返回 0 成功。
pub fn file_attr_set_tag(name: &str, tag: &str) -> isize {
    sys_file_attr_set(name.as_ptr(), name.len(), tag.as_ptr(), tag.len())
}

/// 删除文件的指定标签（属性的"删除"）。
/// 返回 1 = 删掉了已存在的标签；0 = 本不存在；<0 = 出错。
pub fn file_attr_del_tag(name: &str, tag: &str) -> isize {
    sys_file_attr_del(name.as_ptr(), name.len(), tag.as_ptr(), tag.len())
}

/// 删除文件的全部属性。返回 1 = 该文件原本存在；0 = 不存在。
pub fn file_attr_del_all(name: &str) -> isize {
    sys_file_attr_del(name.as_ptr(), name.len(), core::ptr::null(), 0)
}

// ============ 调度优先级 API（任务五 · 多 Agent 协调）============

/// 设置当前任务的调度优先级（数值越大越优先，内核钳到 255）。
/// 返回设置后的优先级值。
pub fn agent_set_priority(priority: usize) -> isize {
    sys_agent_set_priority(priority)
}

/// 任务四性能基准：在 `n` 个文件上把同一组合查询重复 `iters` 次，返回**总耗时(纳秒)**。
/// `use_index=true` 走倒排索引，`false` 走全量扫描。计时在内核内完成，
/// 不含 syscall 往返与序列化开销，能真实反映"索引 vs 遍历"的复杂度差异。
pub fn file_attr_bench(n: usize, iters: usize, use_index: bool) -> isize {
    sys_file_attr_bench(n, iters, if use_index { 1 } else { 0 })
}

/// 从 Path Buffer 直接读取某节点（零拷贝）
///
/// 返回 `(req_bytes, resp_bytes)` 引用——这些数据驻留在用户共享内存里。
pub fn read_path_node_zero_copy(meta: &PathNodeMeta) -> Option<(&'static [u8], &'static [u8])> {
    // Path Buffer 起始：CONTEXT_BASE + layout::PATH_BUFFER_OFF (0x4100)
    const PATH_BUFFER_OFF: usize = 0x4100;
    const NODE_HEADER_SIZE: usize = 8 + 8 + 4 + 4;
    if (meta.len as usize) < NODE_HEADER_SIZE {
        return None;
    }
    let base = AGENT_CONTEXT_BASE + PATH_BUFFER_OFF + meta.offset as usize;
    unsafe {
        let header = core::slice::from_raw_parts(base as *const u8, NODE_HEADER_SIZE);
        let req_len = u32::from_le_bytes([header[16], header[17], header[18], header[19]]) as usize;
        let resp_len = u32::from_le_bytes([header[20], header[21], header[22], header[23]]) as usize;
        let req = core::slice::from_raw_parts((base + NODE_HEADER_SIZE) as *const u8, req_len);
        let resp = core::slice::from_raw_parts(
            (base + NODE_HEADER_SIZE + req_len) as *const u8,
            resp_len,
        );
        Some((req, resp))
    }
}
