#![no_std]
#![no_main]

//! 任务六综合演示：NPC 生态系统
//!
//! 主进程 fork 3 个子进程，每个 exec `agent_npc_worker`。
//! 3 个 NPC Agent 同时运行，互相通过 `send_message` 工具问候，
//! 把自己的"思考路径"写入 Context Path。
//!
//! 整合的子系统：
//! - 任务一：每个 NPC 用 `agent_create` 升级
//! - 任务二：`tool_call(SendMessage / QueryProcess)`
//! - 任务三：`context_push` + `context_query_meta` + 零拷贝读 Path Buffer
//! - 任务五：心跳触发 + `agent_wait` + mailbox

#[macro_use]
extern crate user_lib;

use user_lib::{exec, fork, wait};

const N_NPC: usize = 3;

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    println!("[orchestrator] ===== Task 6: Agent-driven NPC ecosystem =====");
    println!("[orchestrator] goal: integrate Task 1+2+3+5 - {} NPC Agents run concurrently,", N_NPC);
    println!("[orchestrator]       greet each other via tool calls, and record their");
    println!("[orchestrator]       'thinking' into per-Agent context paths.");
    println!("[orchestrator] STEP 1: fork+exec {} NPC worker processes...", N_NPC);
    for i in 0..N_NPC {
        let pid = fork();
        if pid == 0 {
            // child
            exec("agent_npc_worker\0");
            return 0; // exec 失败才会到这里
        } else {
            println!("[orchestrator]   spawned NPC #{} as pid={}", i, pid);
        }
    }

    // 等所有子进程
    println!("[orchestrator] STEP 2: wait for all NPCs to finish their Agent Loops...");
    let mut done = 0;
    while done < N_NPC {
        let mut ec: i32 = 0;
        let p = wait(&mut ec);
        if p > 0 {
            println!(
                "[orchestrator]   NPC pid={} exited with code={}",
                p, ec
            );
            done += 1;
        }
    }
    println!("[orchestrator] result: all {} NPCs done. ecosystem run complete.", N_NPC);
    0
}
