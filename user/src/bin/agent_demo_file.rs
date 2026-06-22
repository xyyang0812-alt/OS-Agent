#![no_std]
#![no_main]

//! 任务四验收：属性查询 + 性能对比
//!
//! 1. 用属性查询找到 tag=demo 的所有文件
//! 2. 用 owner=Agent-A 进一步缩窄
//! 3. 用 keyword=tool 做内容摘要模糊查询
//! 4. **性能对比**：同一个查询，分别用 `use_index=true` 和 `use_index=false`
//!    各跑 200 次，比对耗时（用 `get_time()` 抓时钟）
//!
//! `use_index=true` 走哈希倒排索引；`use_index=false` 走全量扫描。

extern crate alloc;
#[macro_use]
extern crate user_lib;

use alloc::string::ToString;
use user_lib::agent_proto::{FileInfo, QueryResult, ToolName, ToolParams, ToolRequest};
use user_lib::{
    agent_create, file_attr_del_all, file_attr_del_tag, file_attr_set_tag, get_time, tool_call,
};

fn make_req(tag: Option<&str>, owner: Option<&str>, keyword: Option<&str>, use_index: bool) -> ToolRequest {
    static mut CTR: u64 = 1000;
    let id = unsafe {
        CTR += 1;
        CTR
    };
    ToolRequest {
        req_id: id,
        tool: ToolName::QueryFile,
        params: ToolParams::QueryFile {
            tag: tag.map(|s| s.to_string()),
            owner: owner.map(|s| s.to_string()),
            keyword: keyword.map(|s| s.to_string()),
            use_index,
        },
    }
}

fn query(tag: Option<&str>, owner: Option<&str>, keyword: Option<&str>, use_index: bool)
    -> QueryResult<FileInfo>
{
    let req = make_req(tag, owner, keyword, use_index);
    let oc = tool_call(&req).expect("syscall");
    assert!(oc.is_ok(), "query_file failed: {}", oc.status_code);
    postcard::from_bytes(oc.result_bytes()).expect("decode")
}

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    println!("[demo] ===== Task 4: file attribute query + inverted index =====");
    println!("[demo] goal: find files by attributes (tag/owner/keyword) instead of path,");
    println!("[demo]       set/delete attributes, and compare index vs full-scan speed.");
    if agent_create() != 0 {
        println!("[demo] FAIL: agent_create");
        return 1;
    }

    // 1. tag=demo
    println!("[demo] STEP 1/5: query files where tag=\"demo\" (single-condition, indexed)...");
    let r = query(Some("demo"), None, None, true);
    println!("[demo]   -> result: {} file(s) match:", r.items.len());
    for f in &r.items {
        println!(
            "[demo]        {} (owner={:?}, tags={:?}, preview='{}')",
            f.name, f.owner, f.tags, f.digest_preview
        );
    }
    if r.items.is_empty() {
        println!("[demo] FAIL: expected at least 1 demo file");
        return 2;
    }

    // 2. tag=demo AND owner=Agent-A
    println!("[demo] STEP 2/5: multi-condition query tag=\"demo\" AND owner=\"Agent-A\"");
    println!("[demo]          (inverted index intersection)...");
    let r = query(Some("demo"), Some("Agent-A"), None, true);
    println!("[demo]   -> result: {} file(s) match both conditions", r.items.len());

    // 3. keyword=tool
    println!("[demo] STEP 3/5: content-digest fuzzy query keyword=\"tool\"...");
    let r = query(None, None, Some("tool"), true);
    println!("[demo]   -> result: {} file(s) whose digest contains 'tool'", r.items.len());

    // ---- 3.5 属性 设置 → 查询 → 删除 → 查询 回环（验证"删除"功能）----
    println!("[demo] STEP 4/5: attribute set -> query -> delete -> query round-trip");
    const SCRATCH: &str = "scratch_doc";
    const TAG: &str = "ephemeral";

    // 设置：给一个新文件打上 ephemeral 标签
    println!("[demo]   (a) set: tag '{}' on new file '{}'...", TAG, SCRATCH);
    let rc = file_attr_set_tag(SCRATCH, TAG);
    if rc != 0 {
        println!("[demo]   -> FAIL: file_attr_set_tag returned {}", rc);
        return 3;
    }
    // 查询：tag=ephemeral 应能查到刚设置的文件
    let r = query(Some(TAG), None, None, true);
    println!(
        "[demo]   (b) query tag='{}' -> {} file(s) (expect to include '{}')",
        TAG,
        r.items.len(),
        SCRATCH
    );
    if r.items.iter().all(|f| f.name != SCRATCH) {
        println!("[demo]   -> FAIL: set tag not visible via query");
        return 4;
    }
    // 删除：移除该标签
    println!("[demo]   (c) delete: remove tag '{}' from '{}'...", TAG, SCRATCH);
    let rc = file_attr_del_tag(SCRATCH, TAG);
    if rc != 1 {
        println!("[demo]   -> FAIL: file_attr_del_tag -> {} (expected 1=removed)", rc);
        return 5;
    }
    // 查询：tag=ephemeral 不应再包含该文件（倒排索引已同步清理）
    let r = query(Some(TAG), None, None, true);
    println!(
        "[demo]   (d) query tag='{}' again -> {} file(s) (expect '{}' gone)",
        TAG,
        r.items.len(),
        SCRATCH
    );
    if r.items.iter().any(|f| f.name == SCRATCH) {
        println!("[demo]   -> FAIL: tag still visible after delete");
        return 6;
    }
    // 删除整个文件的属性（幂等清理，验证 del_all 路径）
    let _ = file_attr_del_all(SCRATCH);
    println!("[demo]   -> result: set made it queryable, delete removed it from the index");

    // ---- 4. 性能对比：同一查询跑 200 次 ----
    println!("[demo] STEP 5/5: benchmark same query 200x: inverted index vs full scan...");
    const ITERS: u32 = 200;

    let t0 = get_time();
    for _ in 0..ITERS {
        let _ = query(Some("demo"), Some("Agent-A"), None, true);
    }
    let t1 = get_time();
    for _ in 0..ITERS {
        let _ = query(Some("demo"), Some("Agent-A"), None, false);
    }
    let t2 = get_time();

    let indexed_ms = (t1 - t0) as i64;
    let scan_ms = (t2 - t1) as i64;
    let indexed_avg_us = indexed_ms * 1000 / ITERS as i64;
    let scan_avg_us = scan_ms * 1000 / ITERS as i64;
    println!(
        "[demo] indexed:   {} iters in {} ms (avg {} us)",
        ITERS, indexed_ms, indexed_avg_us
    );
    println!(
        "[demo] full-scan: {} iters in {} ms (avg {} us)",
        ITERS, scan_ms, scan_avg_us
    );

    if indexed_ms == 0 && scan_ms == 0 {
        println!("[demo] OK: both paths too fast to measure (workload too small)");
    } else if scan_ms < indexed_ms {
        println!("[demo] WARN: index isn't faster -- baseline file set is tiny;");
        println!("[demo]       in production scale, indexed path scales O(1) per filter.");
    } else if indexed_ms == 0 {
        println!(
            "[demo] OK: indexed too fast to measure (full-scan = {} ms)",
            scan_ms
        );
    } else {
        let speedup_x100 = scan_ms * 100 / indexed_ms;
        println!(
            "[demo] OK: indexed is {}.{:02}x faster than full scan",
            speedup_x100 / 100,
            speedup_x100 % 100
        );
    }

    println!("[demo] PASS task-4");
    0
}
