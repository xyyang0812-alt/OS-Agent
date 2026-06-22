#![no_std]
#![no_main]

//! NPC worker：被 `agent_demo_npc` 通过 fork+exec 启动。
//!
//! 行为：
//! 1. `agent_create` —— 升级为 Agent，分得 Context 区
//! 2. 设置 100ms 心跳
//! 3. 进入 Agent Loop，最多 8 个 tick：
//!    a. `agent_wait` 等待心跳或消息
//!    b. 调用 `query_process` 工具找出所有其他 Agent
//!    c. 给每个其他 Agent 发一条 "hi-from-{my_pid}"
//!    d. 收到的所有消息从 mailbox 取出，把"动作 + 结果"push 到 Context Path
//! 4. 退出前打印自己 Context Path 的全部节点（直接零拷贝读 Path Buffer）
//!
//! 该程序同时演示了任务一/二/三/五。

extern crate alloc;
#[macro_use]
extern crate user_lib;

use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;
use user_lib::agent_proto::{
    AgentTypeFilter, ProcessInfo, QueryResult, ToolName, ToolParams, ToolRequest,
};
use user_lib::{
    EVENT_HEARTBEAT, EVENT_MESSAGE, agent_create, agent_heartbeat_set, agent_heartbeat_stop,
    agent_wait, context_push, context_query_meta, getpid, mailbox_recv,
    read_path_node_zero_copy, tool_call,
};

fn query_other_agents(my_pid: u64) -> Vec<ProcessInfo> {
    let req = ToolRequest {
        req_id: 1,
        tool: ToolName::QueryProcess,
        params: ToolParams::QueryProcess {
            status: None,
            ty: AgentTypeFilter::Agent,
        },
    };
    let oc = tool_call(&req).expect("query_process");
    if !oc.is_ok() {
        return Vec::new();
    }
    let r: QueryResult<ProcessInfo> = postcard::from_bytes(oc.result_bytes()).unwrap_or(QueryResult { items: Vec::new() });
    r.items.into_iter().filter(|p| p.pid as u64 != my_pid).collect()
}

fn send_to(pid: u64, payload: &[u8]) -> isize {
    let req = ToolRequest {
        req_id: 2,
        tool: ToolName::SendMessage,
        params: ToolParams::SendMessage {
            target_pid: pid,
            payload: payload.to_vec(),
        },
    };
    let oc = tool_call(&req).expect("send_message");
    oc.status_code
}

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    let my_pid = getpid() as u64;
    println!("[npc-{}] online: booting NPC agent (will upgrade, heartbeat, loop)", my_pid);

    if agent_create() != 0 {
        println!("[npc-{}] FAIL agent_create", my_pid);
        return 1;
    }
    println!("[npc-{}] upgraded to Agent; setting 100ms heartbeat; entering Agent Loop", my_pid);
    agent_heartbeat_set(100);

    let mut tick = 0u32;
    while tick < 8 {
        let cause = agent_wait(500);
        if cause == 0 {
            // 500ms 还没等到事件，可能心跳还没启动，跳过
            continue;
        }
        tick += 1;
        if cause & EVENT_HEARTBEAT != 0 {
            // 找其他 Agent，发问候
            let others = query_other_agents(my_pid);
            let req_summary = format!("tick={} found {} peers", tick, others.len());
            let mut sent_to = Vec::new();
            for p in &others {
                let msg = format!("hi-from-{}", my_pid);
                let r = send_to(p.pid as u64, msg.as_bytes());
                if r == 0 {
                    sent_to.push(p.pid);
                }
            }
            let resp_summary = format!("greeted={:?}", sent_to);
            let _ = context_push(req_summary.as_bytes(), resp_summary.as_bytes());
            println!(
                "[npc-{}] tick {}: heartbeat -> found {} peer(s), greeted pids {:?}",
                my_pid,
                tick,
                others.len(),
                sent_to
            );
        }
        if cause & EVENT_MESSAGE != 0 {
            // 取出所有消息
            let mut received = Vec::new();
            loop {
                let mut buf = [0u8; 64];
                let n = mailbox_recv(&mut buf);
                if n <= 0 {
                    break;
                }
                let s = core::str::from_utf8(&buf[..n as usize])
                    .unwrap_or("?")
                    .to_string();
                received.push(s);
            }
            if !received.is_empty() {
                let req_summary = format!("got {} message(s)", received.len());
                let resp_summary = format!("{:?}", received);
                let _ = context_push(req_summary.as_bytes(), resp_summary.as_bytes());
                println!(
                    "[npc-{}] tick {}: received {} message(s): {:?}",
                    my_pid,
                    tick,
                    received.len(),
                    received
                );
            }
        }
    }

    agent_heartbeat_stop();

    println!(
        "[npc-{}] loop done; dumping my context path (my 'thinking' history):",
        my_pid
    );
    let meta = context_query_meta().unwrap();
    for (i, m) in meta.items.iter().enumerate() {
        let (req, resp) = read_path_node_zero_copy(m).unwrap_or((&[], &[]));
        let req_s = core::str::from_utf8(req).unwrap_or("?");
        let resp_s = core::str::from_utf8(resp).unwrap_or("?");
        println!("[npc-{}] [{}] req='{}'  resp='{}'", my_pid, i, req_s, resp_s);
    }
    println!("[npc-{}] done", my_pid);
    0
}
