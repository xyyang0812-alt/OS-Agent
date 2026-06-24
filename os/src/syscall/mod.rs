//! Implementation of syscalls
//!
//! The single entry point to all system calls, [`syscall()`], is called
//! whenever userspace wishes to perform a system call using the `ecall`
//! instruction. In this case, the processor raises an 'Environment call from
//! U-mode' exception, which is handled as one of the cases in
//! [`crate::trap::trap_handler`].
//!
//! For clarity, each single syscall is implemented as its own function, named
//! `sys_` then the name of the syscall. You can find functions like this in
//! submodules, and you should also implement syscalls this way.
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

// ---- Agent-OS 扩展 syscall（500 起，避开 rCore 既有编号）----
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

mod agent;
mod fs;
mod process;

use agent::*;
use fs::*;
use process::*;
/// handle syscall exception with `syscall_id` and other arguments
///
/// 参数槽位扩展为 6 个（对应 RISC-V a0-a5），方便 Agent 子系统传
/// 4 参（如 sys_tool_call 的 req_ptr/req_len/out_offset_ptr/out_len_ptr）。
/// rCore 既有 syscall 只用前 3 个，行为不变。
pub fn syscall(syscall_id: usize, args: [usize; 6]) -> isize {
    match syscall_id {
        SYSCALL_OPEN => sys_open(args[0] as *const u8, args[1] as u32),
        SYSCALL_CLOSE => sys_close(args[0]),
        SYSCALL_READ => sys_read(args[0], args[1] as *const u8, args[2]),
        SYSCALL_WRITE => sys_write(args[0], args[1] as *const u8, args[2]),
        SYSCALL_EXIT => sys_exit(args[0] as i32),
        SYSCALL_YIELD => sys_yield(),
        SYSCALL_GET_TIME => sys_get_time(),
        SYSCALL_GETPID => sys_getpid(),
        SYSCALL_FORK => sys_fork(),
        SYSCALL_EXEC => sys_exec(args[0] as *const u8),
        SYSCALL_WAITPID => sys_waitpid(args[0] as isize, args[1] as *mut i32),
        SYSCALL_AGENT_CREATE => sys_agent_create(args[0]),
        SYSCALL_AGENT_INFO => sys_agent_info(args[0], args[1], args[2]),
        SYSCALL_TOOL_CALL => sys_tool_call(args[0], args[1], args[2], args[3]),
        SYSCALL_TOOL_LIST => sys_tool_list(args[0], args[1]),
        SYSCALL_CONTEXT_PUSH => sys_context_push(args[0], args[1], args[2], args[3]),
        SYSCALL_CONTEXT_QUERY => sys_context_query(args[0], args[1], args[2], args[3]),
        SYSCALL_CONTEXT_ROLLBACK => sys_context_rollback(args[0]),
        SYSCALL_CONTEXT_CLEAR => sys_context_clear(),
        SYSCALL_AGENT_HEARTBEAT_SET => sys_agent_heartbeat_set(args[0]),
        SYSCALL_AGENT_HEARTBEAT_STOP => sys_agent_heartbeat_stop(),
        SYSCALL_AGENT_WATCH => sys_agent_watch(args[0] as u32, args[1], args[2]),
        SYSCALL_AGENT_WAIT => sys_agent_wait(args[0] as i64),
        SYSCALL_AGENT_UNWATCH => sys_agent_unwatch(args[0]),
        SYSCALL_MAILBOX_RECV => sys_mailbox_recv(args[0], args[1]),
        SYSCALL_AGENT_SET_LOOP_STATE => sys_agent_set_loop_state(args[0] as u32),
        SYSCALL_FILE_ATTR_DEL => sys_file_attr_del(args[0], args[1], args[2], args[3]),
        SYSCALL_FILE_ATTR_SET => sys_file_attr_set(args[0], args[1], args[2], args[3]),
        SYSCALL_AGENT_SET_PRIORITY => sys_agent_set_priority(args[0]),
        SYSCALL_FILE_ATTR_BENCH => sys_file_attr_bench(args[0], args[1], args[2]),
        _ => panic!("Unsupported syscall_id: {}", syscall_id),
    }
}
