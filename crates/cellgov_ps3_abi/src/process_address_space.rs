//! PS3 LV2 user-process virtual address-space layout.
//!
//! Stack regions, RSX MMIO mirror, and SPU reserved mirror as seen
//! from a PPU user-mode process. Values match RPCS3's `vm.cpp` for
//! cross-runner agreement; the choices below the MMIO boundary are
//! CellGov-internal allocator placement, the choices above it
//! (`0xC000_0000`+) are PS3 hardware truth reported via `sys_rsx_*`
//! and SPU thread-group syscalls.

/// PS3 LV2 primary-thread stack base (matches RPCS3 `vm.cpp`).
pub const PS3_PRIMARY_STACK_BASE: u64 = 0xD000_0000;

/// Primary-thread stack size (64 KB).
pub const PS3_PRIMARY_STACK_SIZE: usize = 0x0001_0000;

/// Base of the child-thread stack region, sitting immediately above the
/// primary stack so `ThreadStackAllocator::CHILD_STACK_BASE` addresses
/// land in real guest memory.
pub const PS3_CHILD_STACKS_BASE: u64 = 0xD001_0000;

/// Size of the child-stack region (15 MB).
pub const PS3_CHILD_STACKS_SIZE: usize = 0x00F0_0000;

/// Initial `gpr[1]` on thread entry: 16 bytes below the stack top, the
/// PPC64 ABI backchain+linkage area.
pub const PS3_PRIMARY_STACK_TOP: u64 =
    PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64 - 0x10;

/// PS3 RSX video/local-memory base. Reads return zero, writes fault.
pub const PS3_RSX_BASE: u64 = 0xC000_0000;

/// RSX reservation size (256 MB).
pub const PS3_RSX_SIZE: usize = 0x1000_0000;

/// PS3 SPU-shared / reserved base. Same read-zero / write-fault semantics
/// as [`PS3_RSX_BASE`].
pub const PS3_SPU_RESERVED_BASE: u64 = 0xE000_0000;

/// SPU reservation size (512 MB).
pub const PS3_SPU_RESERVED_SIZE: usize = 0x2000_0000;
