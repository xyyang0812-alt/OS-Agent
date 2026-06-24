//! 任务四：文件属性系统（不侵入 easy-fs）
//!
//! ## 设计取舍
//!
//! 直接修改 easy-fs 给 inode 加属性字段需要改磁盘布局，对 ch6 基线侵入太大，
//! 也会破坏既有 `usertests` 等用例。我们选择**旁路属性表**：
//!
//! - 全局 `FILE_ATTR_STORE`（在内核内存中）：file_name -> attrs
//! - 倒排索引：(attr_key, attr_value) -> [file_name]
//! - 程序通过工具 `query_file` 或专用 syscall 设置/查询属性
//!
//! 代价：属性不会持久化到磁盘（重启丢失）。对于教学/演示与性能对比，足够。
//! 进阶可后续把整个 store 序列化到一个 `attr.db` 文件中作为简易持久化。

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use lazy_static::lazy_static;

use crate::sync::UPSafeCell;

/// 单个文件的属性（key-value，allow 多个 value）
#[derive(Debug, Clone, Default)]
pub struct FileAttrs {
    /// 通用 kv 属性
    pub kv: BTreeMap<String, String>,
    /// 标签（多值）
    pub tags: Vec<String>,
    /// 内容摘要（前 128 字节或自定义关键词列表）
    pub digest: Vec<u8>,
}

/// 全局属性存储 + 倒排索引
pub struct FileAttrStore {
    /// file_name -> FileAttrs
    pub by_name: BTreeMap<String, FileAttrs>,
    /// (key, value) -> 文件名集合
    pub index_kv: BTreeMap<(String, String), Vec<String>>,
    /// tag -> 文件名集合
    pub index_tag: BTreeMap<String, Vec<String>>,
}

impl FileAttrStore {
    pub const fn new() -> Self {
        Self {
            by_name: BTreeMap::new(),
            index_kv: BTreeMap::new(),
            index_tag: BTreeMap::new(),
        }
    }

    /// 设置属性 kv（覆盖式）
    pub fn set_kv(&mut self, name: &str, key: &str, value: &str) {
        let attrs = self.by_name.entry(name.to_string()).or_default();
        if let Some(old) = attrs.kv.insert(key.to_string(), value.to_string()) {
            // 删除旧索引项
            let old_idx_key = (key.to_string(), old);
            if let Some(v) = self.index_kv.get_mut(&old_idx_key) {
                v.retain(|f| f != name);
            }
        }
        let idx_key = (key.to_string(), value.to_string());
        let entry = self.index_kv.entry(idx_key).or_default();
        if !entry.iter().any(|n| n == name) {
            entry.push(name.to_string());
        }
    }

    /// 添加标签
    pub fn add_tag(&mut self, name: &str, tag: &str) {
        let attrs = self.by_name.entry(name.to_string()).or_default();
        if !attrs.tags.iter().any(|t| t == tag) {
            attrs.tags.push(tag.to_string());
        }
        let entry = self.index_tag.entry(tag.to_string()).or_default();
        if !entry.iter().any(|n| n == name) {
            entry.push(name.to_string());
        }
    }

    /// 设置 owner（kv 简写）
    pub fn set_owner(&mut self, name: &str, owner: &str) {
        self.set_kv(name, "owner", owner);
    }

    /// 设置摘要
    pub fn set_digest(&mut self, name: &str, digest: &[u8]) {
        let attrs = self.by_name.entry(name.to_string()).or_default();
        attrs.digest = digest.to_vec();
    }

    /// 删除某个标签：从文件的 tags 与倒排索引 `index_tag` 中同步移除。
    /// 返回 `true` 表示确实删掉了一个已存在的标签。
    pub fn remove_tag(&mut self, name: &str, tag: &str) -> bool {
        let mut removed = false;
        if let Some(attrs) = self.by_name.get_mut(name) {
            let before = attrs.tags.len();
            attrs.tags.retain(|t| t != tag);
            removed = attrs.tags.len() != before;
        }
        if let Some(v) = self.index_tag.get_mut(tag) {
            v.retain(|f| f != name);
            if v.is_empty() {
                self.index_tag.remove(tag);
            }
        }
        removed
    }

    /// 删除某个 kv 属性：从文件的 kv 与倒排索引 `index_kv` 中同步移除。
    /// 返回 `true` 表示确实删掉了一个已存在的属性。
    pub fn remove_kv(&mut self, name: &str, key: &str) -> bool {
        let mut removed_value: Option<String> = None;
        if let Some(attrs) = self.by_name.get_mut(name) {
            removed_value = attrs.kv.remove(key);
        }
        if let Some(old) = removed_value {
            let idx_key = (key.to_string(), old);
            if let Some(v) = self.index_kv.get_mut(&idx_key) {
                v.retain(|f| f != name);
                if v.is_empty() {
                    self.index_kv.remove(&idx_key);
                }
            }
            true
        } else {
            false
        }
    }

    /// 删除一个文件的全部属性（kv + tags + digest），并清理所有倒排索引项。
    /// 返回 `true` 表示该文件原本存在。
    pub fn remove_file(&mut self, name: &str) -> bool {
        let attrs = match self.by_name.remove(name) {
            Some(a) => a,
            None => return false,
        };
        for (key, value) in attrs.kv.iter() {
            let idx_key = (key.clone(), value.clone());
            if let Some(v) = self.index_kv.get_mut(&idx_key) {
                v.retain(|f| f != name);
                if v.is_empty() {
                    self.index_kv.remove(&idx_key);
                }
            }
        }
        for tag in attrs.tags.iter() {
            if let Some(v) = self.index_tag.get_mut(tag) {
                v.retain(|f| f != name);
                if v.is_empty() {
                    self.index_tag.remove(tag);
                }
            }
        }
        true
    }

    /// **索引查询**（任务四的核心：O(1) 哈希取候选 + O(k) 求交集）
    pub fn query_indexed(
        &self,
        tag: Option<&str>,
        owner: Option<&str>,
        keyword: Option<&str>,
    ) -> Vec<String> {
        let mut candidates: Option<Vec<String>> = None;

        // tag 过滤
        if let Some(t) = tag {
            let cs = self.index_tag.get(t).cloned().unwrap_or_default();
            candidates = Some(intersect(candidates, cs));
        }
        // owner 过滤
        if let Some(o) = owner {
            let cs = self
                .index_kv
                .get(&("owner".to_string(), o.to_string()))
                .cloned()
                .unwrap_or_default();
            candidates = Some(intersect(candidates, cs));
        }
        // keyword：摘要中包含子串
        if let Some(k) = keyword {
            let mut hit: Vec<String> = Vec::new();
            // 若已有 candidates，则在 candidates 中扫描；否则全量
            let names: Vec<&String> = match &candidates {
                Some(c) => c.iter().collect(),
                None => self.by_name.keys().collect(),
            };
            for n in names {
                if let Some(a) = self.by_name.get(n) {
                    if memmem(&a.digest, k.as_bytes()) {
                        hit.push(n.clone());
                    }
                }
            }
            candidates = Some(hit);
        }
        candidates.unwrap_or_default()
    }

    /// **全量扫描**（对照组：把每个文件取出来逐条匹配）
    pub fn query_full_scan(
        &self,
        tag: Option<&str>,
        owner: Option<&str>,
        keyword: Option<&str>,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for (name, attrs) in self.by_name.iter() {
            if let Some(t) = tag {
                if !attrs.tags.iter().any(|x| x == t) {
                    continue;
                }
            }
            if let Some(o) = owner {
                if attrs.kv.get("owner").map(String::as_str) != Some(o) {
                    continue;
                }
            }
            if let Some(k) = keyword {
                if !memmem(&attrs.digest, k.as_bytes()) {
                    continue;
                }
            }
            out.push(name.clone());
        }
        out
    }

    pub fn get(&self, name: &str) -> Option<&FileAttrs> {
        self.by_name.get(name)
    }
}

fn intersect(a: Option<Vec<String>>, b: Vec<String>) -> Vec<String> {
    match a {
        None => b,
        Some(prev) => prev.into_iter().filter(|x| b.contains(x)).collect(),
    }
}

fn memmem(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > hay.len() {
        return false;
    }
    hay.windows(needle.len()).any(|w| w == needle)
}

lazy_static! {
    pub static ref FILE_ATTR_STORE: UPSafeCell<FileAttrStore> =
        unsafe { UPSafeCell::new(FileAttrStore::new()) };
}

/// 初始化：填充几个用于 demo / 评分的示例文件
pub fn init_demo_attrs() {
    let mut s = FILE_ATTR_STORE.exclusive_access();
    // 给 rCore 既有用户程序加点属性，方便演示属性查询
    s.add_tag("agent_demo_create", "demo");
    s.add_tag("agent_demo_create", "task-1");
    s.set_owner("agent_demo_create", "Agent-A");
    s.set_digest("agent_demo_create", b"demonstrates agent process creation");

    s.add_tag("agent_demo_tool", "demo");
    s.add_tag("agent_demo_tool", "task-2");
    s.set_owner("agent_demo_tool", "Agent-A");
    s.set_digest("agent_demo_tool", b"shows structured tool calling protocol");

    s.add_tag("agent_demo_path", "demo");
    s.add_tag("agent_demo_path", "task-3");
    s.set_owner("agent_demo_path", "Agent-B");
    s.set_digest("agent_demo_path", b"context path push/query/rollback");

    s.add_tag("agent_demo_file", "demo");
    s.add_tag("agent_demo_file", "task-4");
    s.set_owner("agent_demo_file", "Agent-B");
    s.set_digest("agent_demo_file", b"queries files by tag and content keyword");

    s.add_tag("user_shell", "system");
    s.set_owner("user_shell", "Kernel");
    s.set_digest("user_shell", b"interactive shell entry");
}

/// 任务四性能验收：**内核内基准测试**。
///
/// 为什么单独写一个内核内基准，而不是在用户态循环调用 `tool_call`？
/// 因为用户态每次查询都要陷入内核 + postcard 序列化/反序列化，这部分固定
/// 开销远大于"几条属性匹配"本身，会把"索引 vs 全扫"的真实差距完全淹没。
/// 这里在一个**独立的局部属性表**上直接调用两条查询函数并用时钟 tick 计时，
/// 排除了 syscall 与序列化开销，能真实反映复杂度差异：
///
/// - `query_indexed`：倒排索引取候选 + 求交集，约 O(命中数)，与 N 基本无关
/// - `query_full_scan`：逐文件匹配，O(N)
///
/// 数据集构造：`n` 个文件作为背景噪声（tag=`bg`, owner=`Agent-Bg`），其中少数
/// 命中目标条件（tag=`needle` AND owner=`Agent-Hit`），模拟"大海捞针"——这正是
/// 索引相对全扫的优势场景。
///
/// 参数：`n` 文件总数；`iters` 同一查询重复次数；`use_index` 选择查询路径。
/// 返回：该查询重复 `iters` 次的**总耗时（纳秒）**。
pub fn run_benchmark(n: usize, iters: usize, use_index: bool) -> usize {
    let mut s = FileAttrStore::new();
    let hits = if n >= 8 { 4 } else { n.min(1) };

    // O(N) 直接构造：基准里文件名天然唯一，绕过 add_tag/set_owner 的去重扫描
    // （那两个接口每次插入都 `.any()` 扫一遍索引向量，批量造 N 个文件会退化成
    // O(N²)，纯属构造开销，不应计入也不该拖慢基准）。
    let mut bg_names: Vec<String> = Vec::new();
    let mut needle_names: Vec<String> = Vec::new();
    for i in 0..n {
        let name = alloc::format!("bench_file_{}", i);
        let mut attrs = FileAttrs::default();
        if i < hits {
            // 针：命中目标条件
            attrs.tags.push("needle".to_string());
            attrs.kv.insert("owner".to_string(), "Agent-Hit".to_string());
            needle_names.push(name.clone());
        } else {
            // 背景噪声
            attrs.tags.push("bg".to_string());
            attrs.kv.insert("owner".to_string(), "Agent-Bg".to_string());
            bg_names.push(name.clone());
        }
        s.by_name.insert(name, attrs);
    }
    // 一次性灌入倒排索引
    s.index_tag.insert("bg".to_string(), bg_names.clone());
    s.index_tag.insert("needle".to_string(), needle_names.clone());
    s.index_kv
        .insert(("owner".to_string(), "Agent-Bg".to_string()), bg_names);
    s.index_kv
        .insert(("owner".to_string(), "Agent-Hit".to_string()), needle_names);

    let start = crate::timer::get_time();
    let mut acc: usize = 0;
    for _ in 0..iters {
        let r = if use_index {
            s.query_indexed(Some("needle"), Some("Agent-Hit"), None)
        } else {
            s.query_full_scan(Some("needle"), Some("Agent-Hit"), None)
        };
        // black_box 防止编译器把"结果未被使用"的查询整体优化掉
        acc = acc.wrapping_add(core::hint::black_box(r.len()));
    }
    let elapsed_ticks = crate::timer::get_time() - start;
    let _ = core::hint::black_box(acc);

    // tick -> 纳秒：ticks * 1e9 / CLOCK_FREQ（usize=64bit，不会溢出）
    elapsed_ticks * 1_000_000_000 / crate::config::CLOCK_FREQ
}
