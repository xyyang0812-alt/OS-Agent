use core::arch::asm;

const SYSCALL_OPEN: usize = 56;
const SYSCALL_CLOSE: usize = 57;
const SYSCALL_READ: usize = 63;
const SYSCALL_WRITE: usize = 64;
const SYSCALL_EXIT: usize = 93;
const SYSCALL_YIELD: usize = 124;
const SYSCALL_GET_TIME: usize = 169;
const SYSCALL_GETPID: usize = 172;
const SYSCALL_FORK: usize = 220;
const SYSCALL_EXEC: usize = 221;
const SYSCALL_WAITPID: usize = 260;

// ---- Agent-OS 扩展 syscall ----
const SYSCALL_AGENT_CREATE: usize = 500;
const SYSCALL_AGENT_INFO: usize = 501;
const SYSCALL_TOOL_CALL: usize = 510;
const SYSCALL_TOOL_LIST: usize = 511;
const SYSCALL_CONTEXT_PUSH: usize = 520;
const SYSCALL_CONTEXT_QUERY: usize = 521;
const SYSCALL_CONTEXT_ROLLBACK: usize = 522;
const SYSCALL_CONTEXT_CLEAR: usize = 523;
const SYSCALL_AGENT_HEARTBEAT_SET: usize = 530;
const SYSCALL_AGENT_HEARTBEAT_STOP: usize = 531;
const SYSCALL_AGENT_WATCH: usize = 532;
const SYSCALL_AGENT_WAIT: usize = 533;
const SYSCALL_AGENT_UNWATCH: usize = 534;
const SYSCALL_MAILBOX_RECV: usize = 535;
const SYSCALL_AGENT_SET_LOOP_STATE: usize = 536;
const SYSCALL_FILE_ATTR_DEL: usize = 537;
const SYSCALL_FILE_ATTR_SET: usize = 538;
const SYSCALL_AGENT_SET_PRIORITY: usize = 539;
const SYSCALL_FILE_ATTR_BENCH: usize = 540;

fn syscall(id: usize, args: [usize; 3]) -> isize {
    let mut ret: isize;
    unsafe {
        asm!(
            "ecall",
            inlateout("x10") args[0] => ret,
            in("x11") args[1],
            in("x12") args[2],
            in("x17") id
        );
    }
    ret
}

fn syscall4(id: usize, args: [usize; 4]) -> isize {
    let mut ret: isize;
    unsafe {
        asm!(
            "ecall",
            inlateout("x10") args[0] => ret,
            in("x11") args[1],
            in("x12") args[2],
            in("x13") args[3],
            in("x17") id
        );
    }
    ret
}

pub fn sys_open(path: &str, flags: u32) -> isize {
    syscall(SYSCALL_OPEN, [path.as_ptr() as usize, flags as usize, 0])
}

pub fn sys_close(fd: usize) -> isize {
    syscall(SYSCALL_CLOSE, [fd, 0, 0])
}

pub fn sys_read(fd: usize, buffer: &mut [u8]) -> isize {
    syscall(
        SYSCALL_READ,
        [fd, buffer.as_mut_ptr() as usize, buffer.len()],
    )
}

pub fn sys_write(fd: usize, buffer: &[u8]) -> isize {
    syscall(SYSCALL_WRITE, [fd, buffer.as_ptr() as usize, buffer.len()])
}

pub fn sys_exit(exit_code: i32) -> ! {
    syscall(SYSCALL_EXIT, [exit_code as usize, 0, 0]);
    panic!("sys_exit never returns!");
}

pub fn sys_yield() -> isize {
    syscall(SYSCALL_YIELD, [0, 0, 0])
}

pub fn sys_get_time() -> isize {
    syscall(SYSCALL_GET_TIME, [0, 0, 0])
}

pub fn sys_getpid() -> isize {
    syscall(SYSCALL_GETPID, [0, 0, 0])
}

pub fn sys_fork() -> isize {
    syscall(SYSCALL_FORK, [0, 0, 0])
}

pub fn sys_exec(path: &str) -> isize {
    syscall(SYSCALL_EXEC, [path.as_ptr() as usize, 0, 0])
}

pub fn sys_waitpid(pid: isize, exit_code: *mut i32) -> isize {
    syscall(SYSCALL_WAITPID, [pid as usize, exit_code as usize, 0])
}

// ============ Agent-OS Syscall Wrappers ============

pub fn sys_agent_create(cfg_ptr: usize) -> isize {
    syscall(SYSCALL_AGENT_CREATE, [cfg_ptr, 0, 0])
}

pub fn sys_agent_info(pid: usize, info_ptr: *mut u8, info_len: usize) -> isize {
    syscall(SYSCALL_AGENT_INFO, [pid, info_ptr as usize, info_len])
}

pub fn sys_tool_call(
    req_ptr: *const u8,
    req_len: usize,
    out_offset: *mut u32,
    out_len: *mut u32,
) -> isize {
    syscall4(
        SYSCALL_TOOL_CALL,
        [
            req_ptr as usize,
            req_len,
            out_offset as usize,
            out_len as usize,
        ],
    )
}

pub fn sys_tool_list(buf_ptr: *mut u8, buf_len: usize) -> isize {
    syscall(SYSCALL_TOOL_LIST, [buf_ptr as usize, buf_len, 0])
}

pub fn sys_context_push(
    req_ptr: *const u8,
    req_len: usize,
    resp_ptr: *const u8,
    resp_len: usize,
) -> isize {
    syscall4(
        SYSCALL_CONTEXT_PUSH,
        [
            req_ptr as usize,
            req_len,
            resp_ptr as usize,
            resp_len,
        ],
    )
}

pub fn sys_context_query(
    start: usize,
    count: usize,
    out_offset: *mut u32,
    out_len: *mut u32,
) -> isize {
    syscall4(
        SYSCALL_CONTEXT_QUERY,
        [start, count, out_offset as usize, out_len as usize],
    )
}

pub fn sys_context_rollback(node_idx: usize) -> isize {
    syscall(SYSCALL_CONTEXT_ROLLBACK, [node_idx, 0, 0])
}

pub fn sys_context_clear() -> isize {
    syscall(SYSCALL_CONTEXT_CLEAR, [0, 0, 0])
}

pub fn sys_agent_heartbeat_set(interval_ms: usize) -> isize {
    syscall(SYSCALL_AGENT_HEARTBEAT_SET, [interval_ms, 0, 0])
}

pub fn sys_agent_heartbeat_stop() -> isize {
    syscall(SYSCALL_AGENT_HEARTBEAT_STOP, [0, 0, 0])
}

pub fn sys_agent_watch(event_mask: u32) -> isize {
    syscall(SYSCALL_AGENT_WATCH, [event_mask as usize, 0, 0])
}

pub fn sys_agent_unwatch(event_mask: u32) -> isize {
    syscall(SYSCALL_AGENT_UNWATCH, [event_mask as usize, 0, 0])
}

pub fn sys_agent_wait(timeout_ms: i64) -> isize {
    syscall(SYSCALL_AGENT_WAIT, [timeout_ms as usize, 0, 0])
}

pub fn sys_mailbox_recv(buf: *mut u8, buf_len: usize) -> isize {
    syscall(SYSCALL_MAILBOX_RECV, [buf as usize, buf_len, 0])
}

pub fn sys_agent_set_loop_state(state_code: u32) -> isize {
    syscall(SYSCALL_AGENT_SET_LOOP_STATE, [state_code as usize, 0, 0])
}

pub fn sys_file_attr_set(
    name_ptr: *const u8,
    name_len: usize,
    tag_ptr: *const u8,
    tag_len: usize,
) -> isize {
    syscall4(
        SYSCALL_FILE_ATTR_SET,
        [name_ptr as usize, name_len, tag_ptr as usize, tag_len],
    )
}

pub fn sys_file_attr_del(
    name_ptr: *const u8,
    name_len: usize,
    tag_ptr: *const u8,
    tag_len: usize,
) -> isize {
    syscall4(
        SYSCALL_FILE_ATTR_DEL,
        [name_ptr as usize, name_len, tag_ptr as usize, tag_len],
    )
}

pub fn sys_agent_set_priority(priority: usize) -> isize {
    syscall(SYSCALL_AGENT_SET_PRIORITY, [priority, 0, 0])
}

pub fn sys_file_attr_bench(n: usize, iters: usize, use_index: usize) -> isize {
    syscall(SYSCALL_FILE_ATTR_BENCH, [n, iters, use_index])
}
