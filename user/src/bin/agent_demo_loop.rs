#![no_std]
#![no_main]

//! 任务五验收：心跳 + 事件驱动 + Agent Loop
//!
//! 1. 设置心跳周期 100 ms
//! 2. 进入 Agent Loop：每次 `agent_wait` 返回都打印一行
//! 3. 在第 3 次心跳后，自己给自己发消息，下一次 wait 应该看到 EVENT_MESSAGE
//! 4. 收到 5 次心跳后停止心跳并 exit

extern crate alloc;
#[macro_use]
extern crate user_lib;

use user_lib::agent_proto::{ToolName, ToolParams, ToolRequest};
use user_lib::{
    EVENT_HEARTBEAT, EVENT_MESSAGE, agent_create, agent_heartbeat_set, agent_heartbeat_stop,
    agent_wait, getpid, mailbox_recv, tool_call,
};

fn send_self(payload: &[u8]) {
    let req = ToolRequest {
        req_id: 9999,
        tool: ToolName::SendMessage,
        params: ToolParams::SendMessage {
            target_pid: getpid() as u64,
            payload: payload.to_vec(),
        },
    };
    let _ = tool_call(&req);
}

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    println!("[demo] ===== Task 5: Agent Loop (heartbeat + event-driven + true sleep) =====");
    println!("[demo] goal: run a think->act->observe loop driven by heartbeat/message");
    println!("[demo]       events; while waiting, the Agent TRULY sleeps (off ready queue).");
    if agent_create() != 0 {
        println!("[demo] FAIL: agent_create");
        return 1;
    }
    println!("[demo] SETUP: Agent created; registering a 100ms heartbeat...");
    agent_heartbeat_set(100);
    println!("[demo]   -> heartbeat interval = 100ms");
    println!("[demo] ENTER Agent Loop: each iter calls agent_wait(500ms) and blocks until");
    println!("[demo]   a heartbeat/message fires (or timeout). Target: 5 heartbeats.");

    let mut hb_count = 0u32;
    let mut msg_count = 0u32;
    let mut iterations = 0u32;

    while hb_count < 5 && iterations < 100 {
        iterations += 1;
        let cause = agent_wait(500);
        if cause == 0 {
            println!("[demo]   iter {}: wait timed out (no event within 500ms)", iterations);
            continue;
        }
        if cause & EVENT_HEARTBEAT != 0 {
            hb_count += 1;
            println!(
                "[demo]   iter {}: woke on HEARTBEAT (#{}/5), cause mask={:#x}",
                iterations, hb_count, cause
            );
            if hb_count == 3 {
                // 给自己投递一条消息
                send_self(b"hello-from-self");
                println!("[demo]     action: at heartbeat #3, sent a message to self via tool");
            }
        }
        if cause & EVENT_MESSAGE != 0 {
            let mut buf = [0u8; 64];
            let n = mailbox_recv(&mut buf);
            if n > 0 {
                msg_count += 1;
                let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("?");
                println!(
                    "[demo]   iter {}: woke on MESSAGE, mailbox payload='{}'",
                    iterations, s
                );
            }
        }
    }

    agent_heartbeat_stop();
    println!("[demo] EXIT loop: stopped heartbeat.");
    println!(
        "[demo] result: {} heartbeats, {} message(s) received over {} loop iterations",
        hb_count, msg_count, iterations
    );
    if hb_count >= 5 && msg_count >= 1 {
        println!("[demo] PASS task-5");
        0
    } else {
        println!("[demo] FAIL: incomplete (need >=5 heartbeats and >=1 message)");
        2
    }
}
