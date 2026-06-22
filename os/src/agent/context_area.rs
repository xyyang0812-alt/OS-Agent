//! Agent Context 区：内核分配、映射到用户空间的共享内存
//!
//! 布局（参见 `docs/design/00-overview.md` §4）：
//!
//! ```text
//! 偏移 0x0000  ┌───────────────────────────┐
//!             │ Header (256B)             │
//! 0x0100      ├───────────────────────────┤
//!             │ Tool Result Ring (16 KB)  │
//! 0x4100      ├───────────────────────────┤
//!             │ Context Path Buffer (32K) │
//! 0xC100      ├───────────────────────────┤
//!             │ Tool Call History (8 KB)  │
//! 0xE100      ├───────────────────────────┤
//!             │ Reserved                  │
//!             └───────────────────────────┘
//! ```

use crate::mm::{MapPermission, MemorySet, VirtAddr, translated_byte_buffer};

/// Agent Context 区的固定虚拟基址（4 GB 处，远离 ELF 段与栈）
pub const CONTEXT_AREA_BASE: usize = 0x8000_0000;

/// 默认大小：64 KB（16 个 4 KB 页）
pub const CONTEXT_AREA_DEFAULT_SIZE: usize = 64 * 1024;

/// 区段切分
pub mod layout {
    pub const HEADER_OFF: usize = 0x0000;
    pub const HEADER_LEN: usize = 0x0100;       // 256 B

    pub const TOOL_RESULT_OFF: usize = 0x0100;
    pub const TOOL_RESULT_LEN: usize = 0x4000;  // 16 KB

    pub const PATH_BUFFER_OFF: usize = 0x4100;
    pub const PATH_BUFFER_LEN: usize = 0x8000;  // 32 KB

    pub const HISTORY_OFF: usize = 0xC100;
    pub const HISTORY_LEN: usize = 0x2000;      // 8 KB
}

/// Header 结构（位于 Context Area 偏移 0），用户态 volatile 读取
#[repr(C)]
#[derive(Debug)]
pub struct AreaHeader {
    pub magic: u32,
    pub version: u32,
    /// 写入序号（seqlock 用，奇数 = 写入中）
    pub seq_number: u64,
    pub tool_result_off: u32,
    pub tool_result_len: u32,
    pub tool_result_write_pos: u32, // 环形缓冲写指针
    pub path_buffer_off: u32,
    pub path_buffer_len: u32,
    pub path_used_bytes: u32,
    pub path_node_count: u32,
    pub history_off: u32,
    pub history_len: u32,
    /// 保留以填充至 256 字节
    pub _reserved: [u8; 256 - 64],
}

pub const HEADER_MAGIC: u32 = 0xA9E4_5EC0;
pub const HEADER_VERSION: u32 = 1;

/// Agent Context 区描述符（在内核 PCB 中保存映射元信息）
pub struct AgentContextArea {
    pub base: VirtAddr,
    pub size: usize,
}

impl AgentContextArea {
    /// 在给定的 MemorySet 中分配并映射 Context 区
    ///
    /// - 大小按页对齐，至少 1 页
    /// - 映射权限：R | W | U（用户可读写——演示阶段简化；
    ///   后续可改成"前 3 段 R|U、最后 1 段 R|W|U"，但需要拆成多个 MapArea，
    ///   暂留 TODO）
    pub fn allocate(memory_set: &mut MemorySet, size: usize) -> Self {
        let aligned = ((size + 0xFFF) & !0xFFF).max(0x1000);
        let base = VirtAddr::from(CONTEXT_AREA_BASE);
        let end = VirtAddr::from(CONTEXT_AREA_BASE + aligned);
        memory_set.insert_framed_area(
            base,
            end,
            MapPermission::R | MapPermission::W | MapPermission::U,
        );
        Self {
            base,
            size: aligned,
        }
    }

    /// 初始化 Header（由内核写入用户态共享内存）
    ///
    /// 通过 `translated_byte_buffer` 跨地址空间访问目标进程的 user 页。
    pub fn init_header(&self, memory_set: &MemorySet) {
        let header = AreaHeader {
            magic: HEADER_MAGIC,
            version: HEADER_VERSION,
            seq_number: 0,
            tool_result_off: layout::TOOL_RESULT_OFF as u32,
            tool_result_len: layout::TOOL_RESULT_LEN as u32,
            tool_result_write_pos: 0,
            path_buffer_off: layout::PATH_BUFFER_OFF as u32,
            path_buffer_len: layout::PATH_BUFFER_LEN as u32,
            path_used_bytes: 0,
            path_node_count: 0,
            history_off: layout::HISTORY_OFF as u32,
            history_len: layout::HISTORY_LEN as u32,
            _reserved: [0u8; 256 - 64],
        };

        let bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(
                &header as *const AreaHeader as *const u8,
                core::mem::size_of::<AreaHeader>(),
            )
        };

        write_user_bytes(
            memory_set.token(),
            self.base.0,
            bytes,
        );
    }

    /// 写入 Tool Result Ring，返回 `(offset_from_area_base, len)`。
    ///
    /// 写入策略：当前 write_pos + len 超过环大小时，直接 wrap to 0（简化的环形缓冲）。
    /// 同时更新 Header 的 `tool_result_write_pos` 字段，方便用户态调试。
    pub fn write_tool_result(
        &self,
        memory_set: &MemorySet,
        data: &[u8],
    ) -> Option<(u32, u32)> {
        if data.len() > layout::TOOL_RESULT_LEN {
            return None; // 单次写入超出整个环
        }
        let token = memory_set.token();

        // 读 header.tool_result_write_pos
        let mut header_buf = [0u8; 256];
        read_user_bytes(token, self.base.0, &mut header_buf);
        let cur_pos = u32::from_le_bytes([
            header_buf[20],
            header_buf[21],
            header_buf[22],
            header_buf[23],
        ]) as usize;

        let mut start = cur_pos;
        if start + data.len() > layout::TOOL_RESULT_LEN {
            start = 0;
        }
        let write_addr = self.base.0 + layout::TOOL_RESULT_OFF + start;
        write_user_bytes(token, write_addr, data);

        // 更新 header.tool_result_write_pos & seq_number
        let new_pos = (start + data.len()) as u32;
        let seq = u64::from_le_bytes([
            header_buf[8],
            header_buf[9],
            header_buf[10],
            header_buf[11],
            header_buf[12],
            header_buf[13],
            header_buf[14],
            header_buf[15],
        ]) + 2; // +2 保持偶数 (seqlock 简化版)

        let mut hdr_patch = [0u8; 16];
        hdr_patch[0..8].copy_from_slice(&seq.to_le_bytes());
        // patch 后面 4 字节 = pos
        hdr_patch[8..12].copy_from_slice(&new_pos.to_le_bytes());
        // 写 seq @ offset 8
        write_user_bytes(token, self.base.0 + 8, &hdr_patch[0..8]);
        // 写 pos @ offset 20
        write_user_bytes(token, self.base.0 + 20, &hdr_patch[8..12]);

        let offset_in_area = (layout::TOOL_RESULT_OFF + start) as u32;
        Some((offset_in_area, data.len() as u32))
    }
}

/// 跨地址空间写入用户态字节（小工具，给本子系统其它模块复用）
pub fn write_user_bytes(token: usize, user_va: usize, data: &[u8]) {
    let chunks = translated_byte_buffer(token, user_va as *const u8, data.len());
    let mut written = 0usize;
    for chunk in chunks {
        let n = chunk.len().min(data.len() - written);
        chunk[..n].copy_from_slice(&data[written..written + n]);
        written += n;
        if written >= data.len() {
            break;
        }
    }
}

/// 跨地址空间读取用户态字节
pub fn read_user_bytes(token: usize, user_va: usize, buf: &mut [u8]) {
    let chunks = translated_byte_buffer(token, user_va as *const u8, buf.len());
    let mut read = 0usize;
    for chunk in chunks {
        let n = chunk.len().min(buf.len() - read);
        buf[read..read + n].copy_from_slice(&chunk[..n]);
        read += n;
        if read >= buf.len() {
            break;
        }
    }
}
