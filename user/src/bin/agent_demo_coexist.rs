#![no_std]
#![no_main]

//! 任务一验收第 3 条：**普通进程和 Agent 进程可共存，互不影响**。
//!
//! 演示思路：
//! 1. 主进程不升级 Agent，保持"普通进程"身份
//! 2. fork 一个子进程 A，**不升级**，做几次 `getpid` / `get_time` / 打印
//! 3. fork 另一个子进程 B，**升级为 Agent**，调用 `tool_call(system_status)`、
//!    `context_push`，最后 exit
//! 4. 主进程 wait 两个子进程，验证：
//!    - 子进程 A 退出码 = 0（普通进程跑得动）
//!    - 子进程 B 退出码 = 0（Agent 进程跑得动）
//!    - 主进程从头到尾**没调 agent_create**，自己也能 wait 子进程
//!
//! 同时主进程在 wait 之前调一次 `agent_info()`，预期 **失败**（-4），
//! 证明它本身不是 Agent 进程——这就证明了"两种进程并存且互不影响"。

extern crate alloc;
#[macro_use]
extern crate user_lib;

use user_lib::agent_proto::{ToolName, ToolParams, ToolRequest};
use user_lib::{
    agent_create, agent_info, context_clear, context_push, exit, fork, get_time, getpid,
    tool_call, wait,
};

fn child_normal() -> i32 {
    println!("[child-normal pid={}] hello, I am a plain process", getpid());
    // 普通进程也能用基础 syscall
    let t = get_time();
    println!("[child-normal pid={}] get_time -> {}ms", getpid(), t);
    // 但普通进程**不能**调 Agent 专属 syscall
    match agent_info() {
        Err(-4) => println!("[child-normal pid={}] OK: agent_info refused (-4)", getpid()),
        Err(code) => {
            println!(
                "[child-normal pid={}] FAIL: expected -4, got {}",
                getpid(),
                code
            );
            return 1;
        }
        Ok(_) => {
            println!("[child-normal pid={}] FAIL: agent_info returned Ok!?", getpid());
            return 2;
        }
    }
    println!("[child-normal pid={}] exit 0", getpid());
    0
}

fn child_agent() -> i32 {
    println!("[child-agent pid={}] starting", getpid());
    if agent_create() != 0 {
        println!("[child-agent pid={}] FAIL agent_create", getpid());
        return 10;
    }
    let info = agent_info().unwrap();
    println!(
        "[child-agent pid={}] OK agent_info: ty={}, size={}",
        getpid(),
        info.agent_type,
        info.context_area_size
    );
    // 用一次工具
    let req = ToolRequest {
        req_id: 1,
        tool: ToolName::SystemStatus,
        params: ToolParams::SystemStatus,
    };
    let oc = tool_call(&req).expect("syscall");
    if !oc.is_ok() {
        println!("[child-agent pid={}] FAIL tool_call -> {}", getpid(), oc.status_code);
        return 11;
    }
    println!("[child-agent pid={}] OK tool_call(system_status)", getpid());
    // 写一条 Context Path
    let _ = context_push(b"step-1", b"saw-system-status");
    let _ = context_clear();
    println!("[child-agent pid={}] exit 0", getpid());
    0
}

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    println!("[demo] ===== Task 1 (coexistence): normal & Agent processes side by side =====");
    println!("[demo] goal: a normal process and an Agent process run together without");
    println!("[demo]       interfering; only Agents may use Agent-only syscalls.");
    println!("[parent pid={}] STEP 1/5: parent stays a NORMAL process", getpid());

    // 1. parent 是普通进程，agent_info() 必须失败
    match agent_info() {
        Err(-4) => println!("[parent] OK: parent is NOT an Agent (agent_info -> -4)"),
        other => {
            println!("[parent] FAIL: parent unexpected agent_info: {:?}", other);
            return 1;
        }
    }

    // 2. fork 普通子进程
    let pid_a = fork();
    if pid_a == 0 {
        exit(child_normal());
    }
    println!("[parent] forked normal child as pid={}", pid_a);

    // 3. fork Agent 子进程
    let pid_b = fork();
    if pid_b == 0 {
        exit(child_agent());
    }
    println!("[parent] forked agent  child as pid={}", pid_b);

    // 4. parent 自己仍然是普通进程，再校验一次
    match agent_info() {
        Err(-4) => println!("[parent] OK: parent still NOT an Agent after forks"),
        other => {
            println!("[parent] FAIL: parent unexpected agent_info: {:?}", other);
            return 2;
        }
    }

    // 5. 等两个子进程
    let mut done = 0;
    let mut all_ok = true;
    while done < 2 {
        let mut ec: i32 = 0;
        let p = wait(&mut ec);
        if p > 0 {
            let label = if p == pid_a {
                "normal"
            } else if p == pid_b {
                "agent "
            } else {
                "other "
            };
            println!("[parent] {} child pid={} exited code={}", label, p, ec);
            if ec != 0 {
                all_ok = false;
            }
            done += 1;
        }
    }

    if !all_ok {
        println!("[demo] FAIL: at least one child failed");
        return 3;
    }
    println!("[demo] PASS task-1 coexistence");
    0
}
