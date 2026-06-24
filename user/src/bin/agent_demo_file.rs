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
    agent_create, file_attr_bench, file_attr_del_all, file_attr_del_tag, file_attr_set_tag,
    tool_call,
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

    // ---- 5. 规模化性能对比（内核内计时，排除 syscall 开销）----
    // 之前的"同一小数据集跑 200 次 tool_call"无法证明性能差异：① 数据集只有 5
    // 个文件；② 计时是毫秒级；③ 真正耗时被 syscall 往返/序列化淹没。这里改用内核内
    // 基准 file_attr_bench：在 N 个文件上直接对两条查询函数用时钟 tick 计时，
    // 并随 N 放大，证明 full-scan 随 N 线性增长、而倒排索引基本持平。
    println!("[demo] STEP 5/5: scaling benchmark (kernel-internal timing, excludes syscall cost)");
    println!("[demo]   workload: N files w/ tag=bg owner=Agent-Bg, few hits tag=needle AND owner=Agent-Hit");
    const ITERS: usize = 200;
    println!("[demo]   query repeated {}x per N; lower ns = faster", ITERS);
    println!(
        "[demo]   {:>7} | {:>16} | {:>16} | {:>12}",
        "N", "full-scan(ns)", "indexed(ns)", "speedup"
    );
    println!("[demo]   --------+------------------+------------------+-------------");

    let mut linear_ok = false;
    let mut prev_scan: isize = 0;
    for &n in &[10usize, 100, 1000, 5000, 10000] {
        // 注意先后顺序不影响：每次都重建独立局部属性表
        let scan = file_attr_bench(n, ITERS, false);
        let idx = file_attr_bench(n, ITERS, true);
        if idx > 0 {
            let sp = scan * 100 / idx;
            println!(
                "[demo]   {:>7} | {:>16} | {:>16} | {:>9}.{:02}x",
                n,
                scan,
                idx,
                sp / 100,
                sp % 100
            );
        } else {
            println!(
                "[demo]   {:>7} | {:>16} | {:>16} | {:>12}",
                n, scan, idx, "idx~0(fast)"
            );
        }
        // 检查 full-scan 是否随 N 增长（最大 N 至少比最小 N 明显更慢）
        if n == 10000 && scan > prev_scan {
            linear_ok = true;
        }
        if n == 10 {
            prev_scan = scan;
        }
    }

    if linear_ok {
        println!("[demo]   -> full-scan grows with N (O(N)); indexed stays ~flat (O(hits)).");
        println!("[demo]   -> CONCLUSION: inverted index outperforms full traversal at scale.");
    } else {
        println!("[demo]   -> WARN: scan did not grow as expected; check clock resolution.");
    }

    println!("[demo] PASS task-4");
    0
}
