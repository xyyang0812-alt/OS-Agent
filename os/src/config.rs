//! Constants used in rCore
#[allow(unused)]

pub const USER_STACK_SIZE: usize = 4096 * 2;
pub const KERNEL_STACK_SIZE: usize = 4096 * 2;
// 24 MiB 内核堆：任务四的规模化性能基准（run_benchmark）会在内核内构造上万个
// 文件属性条目（每个文件的 kv BTreeMap 各占约 1KB 节点），需要较大堆空间。
// QEMU virt 提供约 126MB RAM（MEMORY_END - 内核基址），24MB 堆绰绰有余。
pub const KERNEL_HEAP_SIZE: usize = 0x180_0000;

pub const PAGE_SIZE: usize = 0x1000;
pub const PAGE_SIZE_BITS: usize = 0xc;

pub const TRAMPOLINE: usize = usize::MAX - PAGE_SIZE + 1;
pub const TRAP_CONTEXT: usize = TRAMPOLINE - PAGE_SIZE;

pub use crate::board::{CLOCK_FREQ, MEMORY_END, MMIO};
