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
