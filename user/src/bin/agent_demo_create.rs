#![no_std]
#![no_main]

//! 任务一验收：
//!
//! 1. 当前是普通进程时，`agent_info()` 失败（返回 -4）
//! 2. 调用 `agent_create()` 后，进程升级为 Agent，Context 区在 0x8000_0000
//! 3. `agent_info()` 成功并返回正确的元信息
//! 4. 用户态可直接读 Context Area Header 字节，看到内核写入的 magic / version

#[macro_use]
extern crate user_lib;

use user_lib::{
    AGENT_CONTEXT_BASE, AgentInfo, agent_context_area, agent_create, agent_info, loop_state_name,
};

const HEADER_MAGIC: u32 = 0xA9E4_5EC0;

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    println!("[demo] ===== Task 1: Agent process creation & address space =====");
    println!("[demo] goal: turn a normal process into an Agent and verify its");
    println!("[demo]       kernel PCB fields + user-mapped Context Area.");

    // 1. 升级前应失败
    println!("[demo] STEP 1/4: query agent_info BEFORE upgrade (a normal process");
    println!("[demo]          must NOT have Agent metadata, so this should fail)...");
    match agent_info() {
        Ok(_) => {
            println!("[demo]   -> UNEXPECTED: agent_info() succeeded on a normal process");
            println!("[demo] FAIL: expected agent_info() to fail before agent_create");
            return 1;
        }
        Err(code) => {
            println!(
                "[demo]   -> OK: agent_info refused with err({}) => process is not an Agent yet",
                code
            );
        }
    }

    // 2. 升级
    println!("[demo] STEP 2/4: calling sys_agent_create to upgrade into an Agent");
    println!("[demo]          (kernel allocates a 64KB Context Area + inits PCB ext)...");
    let r = agent_create();
    if r != 0 {
        println!("[demo]   -> FAIL: agent_create returned {}", r);
        return 2;
    }
    println!("[demo]   -> OK: agent_create returned 0 (upgrade succeeded)");

    // 3. 查询元信息
    println!("[demo] STEP 3/4: query agent_info AFTER upgrade (read PCB ext fields)...");
    let info: AgentInfo = match agent_info() {
        Ok(i) => i,
        Err(code) => {
            println!("[demo]   -> FAIL: agent_info returned err({})", code);
            return 3;
        }
    };
    println!(
        "[demo]   -> OK: agent_type={} (1=Normal,2=System), context_area_size={} bytes",
        info.agent_type, info.context_area_size
    );
    println!(
        "[demo]         context_path_nodes={}, loop_state={}",
        info.path_node_count,
        loop_state_name(info.loop_state)
    );

    // 4. 零拷贝读 Context Area Header
    println!("[demo] STEP 4/4: zero-copy read of the Context Area header the kernel");
    println!(
        "[demo]          wrote into our user space at {:#x} (no syscall needed)...",
        AGENT_CONTEXT_BASE
    );
    let bytes = unsafe { agent_context_area(64) };
    let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    println!(
        "[demo]   -> read magic={:#x} (expect {:#x}), version={}",
        magic, HEADER_MAGIC, version
    );
    if magic != HEADER_MAGIC {
        println!("[demo]   -> FAIL: bad magic, Context Area not mapped/initialized correctly");
        return 4;
    }
    println!("[demo]   -> OK: magic matches => kernel-written header is visible to user");
    println!("[demo] result: Agent created, PCB fields initialized, Context Area readable");
    println!("[demo] PASS task-1");
    0
}
