//! PS3 LV2 user-process virtual address-space layout.

/// Base guest virtual address of the primary thread's stack region.
pub const PS3_PRIMARY_STACK_BASE: u64 = 0xD000_0000;

/// Size in bytes of the primary thread's stack region. 1 MiB
/// matches what real PS3 titles declare in their `sys_proc_param`
/// (PROC_PARAM.primary_stacksize).
pub const PS3_PRIMARY_STACK_SIZE: usize = 0x0010_0000;

/// Sits immediately above the primary stack so child-stack allocator
/// addresses land in real guest memory.
pub const PS3_CHILD_STACKS_BASE: u64 = 0xD010_0000;

/// Size in bytes of the child-thread stacks region.
pub const PS3_CHILD_STACKS_SIZE: usize = 0x00F0_0000;

/// 16 bytes below stack top reserves the PPC64 ABI backchain+linkage area.
pub const PS3_PRIMARY_STACK_TOP: u64 =
    PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64 - 0x10;

/// Smallest primary stack the kernel hands out, larger than the 4 KiB
/// floor the headers advertise. RPCS3 `PPUModule.cpp` `ppu_load_exec`
/// records the observed floor as 64 KiB.
pub const PS3_PRIMARY_STACK_SIZE_MIN: u32 = 0x1_0000;

/// Largest primary stack the kernel hands out; a declaration above it
/// is clamped, not refused. RPCS3 `sys_process.h`.
pub const PS3_PRIMARY_STACK_SIZE_MAX: u32 = 0x10_0000;

/// Granularity a clamped primary-stack size is rounded up to.
pub const PS3_STACK_SIZE_GRANULARITY: u32 = 0x1000;

/// Base of the iomap region `sys_rsx_context_iomap` (672) maps into;
/// libgcm asks for an IO window starting here.
pub const PS3_RSX_IOMAP_BASE: u64 = 0x4000_0000;

/// Size of the backed iomap region, captured from the `size` argument
/// of a retail title's first `sys_rsx_context_iomap` call.
pub const PS3_RSX_IOMAP_SIZE: usize = 0x0550_0000;

/// RSX video/local-memory MMIO: reads return zero, writes fault.
pub const PS3_RSX_BASE: u64 = 0xC000_0000;

/// Size in bytes of the RSX MMIO region.
pub const PS3_RSX_SIZE: usize = 0x1000_0000;

/// SPU-shared MMIO: same read-zero / write-fault semantics as [`PS3_RSX_BASE`].
pub const PS3_SPU_RESERVED_BASE: u64 = 0xE000_0000;

/// Size in bytes of the SPU-shared MMIO region.
pub const PS3_SPU_RESERVED_SIZE: usize = 0x2000_0000;

/// Lowest plausible address for PS3 LV2 user text: the trampoline
/// scratch zone (`0..0x10000`) sits below it, user heap and title text
/// above. An LV2 convention with no architectural backing.
pub const PS3_USER_TEXT_FLOOR: u64 = 0x0001_0000;

// The boot-composed iomap window must not overlap the RSX MMIO
// region. The sibling regions (primary stack, child stacks, SPU MMIO)
// all sit above PS3_RSX_BASE and so stay disjoint.
const _: () = assert!(PS3_RSX_IOMAP_BASE + PS3_RSX_IOMAP_SIZE as u64 <= PS3_RSX_BASE);
