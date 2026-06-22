#![no_std]
#![no_main]

//! 任务三验收：
//!
//! 1. 升级为 Agent
//! 2. 连续 push 5 个 Context Path 节点
//! 3. 通过 `context_query_meta()` 查到 5 个元信息
//! 4. 通过 `read_path_node_zero_copy()` 直接读 Path Buffer，无 syscall
//! 5. rollback 到第 3 个节点，再次查询应该剩 3 个
//! 6. push 一个新节点，验证 rollback 后空间确实可复用
//! 7. clear 清空，查询应该为 0
//! 8. 触发 LRU 淘汰：用极小的 max_bytes 配额连续 push，验证不 OOM
//!    （配额由 `sys_agent_create` 的默认值 16KB 决定；这里只 push 中等节点即可）

extern crate alloc;
#[macro_use]
extern crate user_lib;

use alloc::format;
use user_lib::{
    agent_create, context_clear, context_push, context_query_meta, context_rollback,
    read_path_node_zero_copy,
};

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    println!("[demo] ===== Task 3: context path management =====");
    println!("[demo] goal: maintain an Agent-Loop query path. Kernel keeps node META");
    println!("[demo]       (PCB); node DATA lives in the user Context Area Path Buffer.");
    if agent_create() != 0 {
        println!("[demo] FAIL: agent_create");
        return 1;
    }
    println!("[demo] SETUP: Agent created (default quota: 128 nodes / 16KB Path Buffer)");

    // 1. 连续 push 5 节点
    println!("[demo] STEP 1/7: push 5 context nodes (one per simulated Agent-Loop round)...");
    for i in 0..5 {
        let req = format!("step-{}-thinking", i);
        let resp = format!("step-{}-result", i);
        let idx = context_push(req.as_bytes(), resp.as_bytes()).expect("push");
        println!(
            "[demo]   pushed node idx={} : req='{}' resp='{}'",
            idx, req, resp
        );
    }

    // 2. 查询元信息
    println!("[demo] STEP 2/7: query path metadata via sys_context_query...");
    let meta = context_query_meta().expect("query");
    println!("[demo]   -> result: path now has {} node(s)", meta.items.len());
    if meta.items.len() != 5 {
        println!("[demo] FAIL: expected 5 nodes");
        return 2;
    }

    // 3. 零拷贝读每个节点（无 syscall）
    println!("[demo] STEP 3/7: zero-copy read each node straight from Path Buffer (no syscall)...");
    for (i, m) in meta.items.iter().enumerate() {
        let (req, resp) = read_path_node_zero_copy(m).expect("read");
        let req_s = core::str::from_utf8(req).unwrap_or("?");
        let resp_s = core::str::from_utf8(resp).unwrap_or("?");
        println!(
            "[demo]   node[{}] seq={} off={}: req='{}', resp='{}'",
            i, m.seq, m.offset, req_s, resp_s
        );
    }

    // 4. rollback 到前 3 个
    println!("[demo] STEP 4/7: rollback to keep only the first 3 nodes...");
    let r = context_rollback(3);
    if r != 0 {
        println!("[demo]   -> FAIL: rollback returned {}", r);
        return 3;
    }
    let meta = context_query_meta().expect("query after rollback");
    if meta.items.len() != 3 {
        println!(
            "[demo]   -> FAIL: expected 3 nodes after rollback, got {}",
            meta.items.len()
        );
        return 4;
    }
    println!("[demo]   -> result: path truncated to {} nodes", meta.items.len());

    // 5. 再 push 一个
    println!("[demo] STEP 5/7: push a new node after rollback (reuses freed space)...");
    let new_idx = context_push(b"after-rollback-req", b"after-rollback-resp").expect("push");
    let meta = context_query_meta().expect("query after re-push");
    println!(
        "[demo]   -> result: new node idx={}, path now {} nodes (expect 4)",
        new_idx,
        meta.items.len()
    );
    if meta.items.len() != 4 {
        println!("[demo] FAIL: expected 4 nodes");
        return 5;
    }

    // 6. clear
    println!("[demo] STEP 6/7: clear the whole path via sys_context_clear...");
    context_clear();
    let meta = context_query_meta().expect("query after clear");
    if !meta.items.is_empty() {
        println!("[demo]   -> FAIL: expected 0 nodes after clear, got {}", meta.items.len());
        return 6;
    }
    println!("[demo]   -> result: path is now empty ({} nodes)", meta.items.len());

    // 7. 触发淘汰：push 多个较大节点（每个约 2 KB），加起来超过 16 KB max_bytes
    println!("[demo] STEP 7/7: stress test eviction - push 20 large (~3KB) nodes to exceed");
    println!("[demo]          the 16KB quota; kernel must auto-evict (LRU) and NOT OOM...");
    let big_payload = [0xABu8; 1500];
    let mut pushed_total = 0usize;
    for i in 0..20 {
        match context_push(&big_payload, &big_payload) {
            Ok(_) => pushed_total += 1,
            Err(code) => {
                println!("[demo]   push #{} rejected with err {}", i, code);
                break;
            }
        }
    }
    let meta = context_query_meta().expect("query after stress");
    println!(
        "[demo]   -> result: attempted {} pushes, path capped at {} nodes (older ones evicted)",
        pushed_total,
        meta.items.len()
    );
    if meta.items.len() >= pushed_total {
        println!("[demo] FAIL: expected eviction to cap node count");
        return 7;
    }

    println!("[demo] result: push/query/zero-copy-read/rollback/clear/eviction all verified");
    println!("[demo] PASS task-3");
    0
}
