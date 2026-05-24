//! PS3 LV2 user-process virtual address-space layout.

/// Base guest virtual address of the primary thread's stack region.
pub const PS3_PRIMARY_STACK_BASE: u64 = 0xD000_0000;

/// Size in bytes of the primary thread's stack region. 1 MiB
/// matches what real PS3 titles declare in their `sys_proc_param`
/// (PROC_PARAM.primary_stacksize); a smaller floor used to alias
/// onto the child-stacks region when a title asked for 1 MiB and
/// the boot.rs check now refuses to truncate.
pub const PS3_PRIMARY_STACK_SIZE: usize = 0x0010_0000;

/// Sits immediately above the primary stack so child-stack allocator
/// addresses land in real guest memory.
pub const PS3_CHILD_STACKS_BASE: u64 = 0xD010_0000;

/// Size in bytes of the child-thread stacks region.
pub const PS3_CHILD_STACKS_SIZE: usize = 0x00F0_0000;

/// 16 bytes below stack top reserves the PPC64 ABI backchain+linkage area.
pub const PS3_PRIMARY_STACK_TOP: u64 =
    PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64 - 0x10;

/// Base of the iomap region `sys_rsx_context_iomap` (672) maps into.
/// libgcm asks for an IO window starting here; backing it as
/// ReadWrite from boot lets the title's FIFO command buffer
/// allocations land in a writable region.
pub const PS3_RSX_IOMAP_BASE: u64 = 0x4000_0000;

/// Size of the backed iomap region. Captured from WipEout's first
/// `sys_rsx_context_iomap` call (the `size` argument); larger
/// requests trip the over-cap diagnostic in the 672 dispatch handler.
pub const PS3_RSX_IOMAP_SIZE: usize = 0x0550_0000;

/// RSX video/local-memory MMIO: reads return zero, writes fault.
pub const PS3_RSX_BASE: u64 = 0xC000_0000;

/// Size in bytes of the RSX MMIO region.
pub const PS3_RSX_SIZE: usize = 0x1000_0000;

/// SPU-shared MMIO: same read-zero / write-fault semantics as [`PS3_RSX_BASE`].
pub const PS3_SPU_RESERVED_BASE: u64 = 0xE000_0000;

/// Size in bytes of the SPU-shared MMIO region.
pub const PS3_SPU_RESERVED_SIZE: usize = 0x2000_0000;

/// Lowest plausible address for PS3 LV2 user text. Below this lives the
/// trampoline scratch zone (`0..0x10000`); above it lives user heap and
/// title text. Diagnostic walks reject candidate return addresses below
/// this floor as obvious junk. OS-level convention, not architectural.
pub const PS3_USER_TEXT_FLOOR: u64 = 0x0001_0000;

// The boot-composed iomap window must not overlap the RSX MMIO
// region. Consumed by the boot region composer that places
// `[PS3_RSX_IOMAP_BASE, +PS3_RSX_IOMAP_SIZE)`; sibling regions
// (primary stack, child stacks, SPU MMIO) sit above PS3_RSX_BASE
// and are layout-disjoint by construction.
const _: () = assert!(PS3_RSX_IOMAP_BASE + PS3_RSX_IOMAP_SIZE as u64 <= PS3_RSX_BASE);
