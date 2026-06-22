#![no_std]
#![no_main]

//! 任务二验收：
//!
//! 1. 升级为 Agent 进程
//! 2. 调用 `system_status` 工具，零拷贝读结果
//! 3. 调用 `query_process` 过滤 Agent 进程
//! 4. 调用 `read_context` 读取自身信息
//! 5. 调用 `send_message`、`query_file` 验证 NotImplemented 状态码正确返回
//! 6. 调用 `tool_list` 拿到工具列表

extern crate alloc;
#[macro_use]
extern crate user_lib;

use alloc::string::ToString;
use user_lib::agent_proto::{
    AgentTypeFilter, ContextTargetType, ProcessInfo, QueryResult, SystemStatusInfo, ToolName,
    ToolParams, ToolRequest,
};
use user_lib::{agent_create, getpid, mailbox_recv, tool_call, tool_list};

fn make_req(tool: ToolName, params: ToolParams) -> ToolRequest {
    static mut COUNTER: u64 = 1;
    let id = unsafe {
        COUNTER += 1;
        COUNTER
    };
    ToolRequest {
        req_id: id,
        tool,
        params,
    }
}

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    println!("[demo] ===== Task 2: structured tool call (Agent <-> kernel) =====");
    println!("[demo] goal: send strongly-typed tool requests; kernel executes them and");
    println!("[demo]       writes results into the Context Area (zero-copy read back).");

    println!("[demo] SETUP: upgrading to an Agent (needed to own a Context Area)...");
    let r = agent_create();
    if r != 0 {
        println!("[demo]   -> FAIL: agent_create -> {}", r);
        return 1;
    }
    println!("[demo]   -> OK: now an Agent");

    // ---- 1. system_status ----
    println!("[demo] STEP 1/6: call tool 'system_status' (no params)...");
    let req = make_req(ToolName::SystemStatus, ToolParams::SystemStatus);
    let oc = match tool_call(&req) {
        Ok(o) => o,
        Err(e) => {
            println!("[demo] FAIL: system_status syscall err {}", e);
            return 2;
        }
    };
    if !oc.is_ok() {
        println!("[demo] FAIL: system_status status_code = {}", oc.status_code);
        return 3;
    }
    let bytes = oc.result_bytes();
    let info: SystemStatusInfo = match postcard::from_bytes(bytes) {
        Ok(i) => i,
        Err(_) => {
            println!("[demo] FAIL: cannot decode system_status result");
            return 4;
        }
    };
    println!(
        "[demo]   -> result: total_procs={}, agent_procs={}, running={}, uptime_ticks={}",
        info.total_procs, info.agent_procs, info.running_procs, info.uptime_ticks
    );

    // ---- 2. query_process: agents only ----
    println!("[demo] STEP 2/6: call tool 'query_process' with filter {{ ty=Agent }}...");
    let req = make_req(
        ToolName::QueryProcess,
        ToolParams::QueryProcess {
            status: None,
            ty: AgentTypeFilter::Agent,
        },
    );
    let oc = tool_call(&req).expect("query_process syscall");
    if !oc.is_ok() {
        println!("[demo]   -> FAIL: query_process status = {}", oc.status_code);
        return 5;
    }
    let result: QueryResult<ProcessInfo> = postcard::from_bytes(oc.result_bytes()).unwrap();
    println!(
        "[demo]   -> result: {} Agent process(es) found:",
        result.items.len()
    );
    for p in &result.items {
        println!(
            "[demo]        pid={}, name={}, is_agent={}, status={:?}",
            p.pid, p.name, p.is_agent, p.status
        );
    }

    // ---- 3. read_context: self ----
    let my_pid = getpid() as u64;
    println!(
        "[demo] STEP 3/6: call tool 'read_context' to read our own info (pid={})...",
        my_pid
    );
    let req = make_req(
        ToolName::ReadContext,
        ToolParams::ReadContext {
            target_type: ContextTargetType::Agent,
            target_id: my_pid,
        },
    );
    let oc = tool_call(&req).expect("read_context syscall");
    if !oc.is_ok() {
        println!("[demo]   -> FAIL: read_context self status = {}", oc.status_code);
        return 6;
    }
    let me: ProcessInfo = postcard::from_bytes(oc.result_bytes()).unwrap();
    println!(
        "[demo]   -> result: pid={}, is_agent={}, status={:?}",
        me.pid, me.is_agent, me.status
    );

    // ---- 4. send_message: 给自己投一条消息，再用 mailbox_recv 取出来 ----
    let my_pid = getpid() as u64;
    println!("[demo] STEP 4/6: call tool 'send_message' to post 'hello-self' to our own");
    println!("[demo]          mailbox, then drain it with sys_mailbox_recv...");
    let req = make_req(
        ToolName::SendMessage,
        ToolParams::SendMessage {
            target_pid: my_pid,
            payload: b"hello-self".to_vec(),
        },
    );
    let oc = tool_call(&req).expect("send_message syscall");
    if !oc.is_ok() {
        println!("[demo]   -> FAIL: send_message -> {}", oc.status_code);
        return 7;
    }
    // mailbox_recv 应当能取出刚投递的消息
    let mut buf = [0u8; 32];
    let n = mailbox_recv(&mut buf);
    if n <= 0 {
        println!("[demo]   -> FAIL: mailbox_recv returned {}", n);
        return 71;
    }
    let got = core::str::from_utf8(&buf[..n as usize]).unwrap_or("?");
    if got != "hello-self" {
        println!("[demo]   -> FAIL: mailbox content mismatch: '{}'", got);
        return 72;
    }
    println!(
        "[demo]   -> result: round-trip OK, received {} bytes = '{}'",
        n, got
    );

    // ---- 5. query_file (task-4 已实现) ----
    println!("[demo] STEP 5/6: call tool 'query_file' with filter {{ tag=\"demo\" }}...");
    let req = make_req(
        ToolName::QueryFile,
        ToolParams::QueryFile {
            tag: Some("demo".to_string()),
            owner: None,
            keyword: None,
            use_index: true,
        },
    );
    let oc = tool_call(&req).expect("query_file syscall");
    if !oc.is_ok() {
        println!("[demo]   -> FAIL: query_file status_code = {}", oc.status_code);
        return 8;
    }
    let qr: QueryResult<user_lib::agent_proto::FileInfo> =
        postcard::from_bytes(oc.result_bytes()).unwrap();
    println!(
        "[demo]   -> result: {} file(s) tagged 'demo' (via inverted index)",
        qr.items.len()
    );

    // ---- 6. tool_list ----
    println!("[demo] STEP 6/6: call sys_tool_list to enumerate available kernel tools...");
    let mut buf = [0u8; 1024];
    let n = match tool_list(&mut buf) {
        Ok(n) => n,
        Err(e) => {
            println!("[demo]   -> FAIL: tool_list err {}", e);
            return 9;
        }
    };
    let list: QueryResult<user_lib::agent_proto::ToolDescriptor> =
        postcard::from_bytes(&buf[..n]).unwrap();
    println!("[demo]   -> result: {} tools registered in kernel:", list.items.len());
    for d in &list.items {
        println!("[demo]        - {}", d.name);
    }

    println!("[demo] result: all 5 tools invoked, results decoded from Context Area");
    println!("[demo] PASS task-2");
    0
}
