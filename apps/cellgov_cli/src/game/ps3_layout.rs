//! PS3 guest-memory layout constants shared across the run-game /
//! bench-boot pipeline.
//!
//! These addresses describe the platform the harness emulates, not
//! any one subcommand's setup. Kept in their own module so a future
//! consumer (a new bench variant, an export, a test fixture) does
//! not have to reach into `game.rs` orchestration to learn "where
//! does the primary-thread stack live."

/// PS3 LV2 primary-thread stack base. Matches RPCS3 `vm.cpp`'s
/// 0xD0000000 page-4K stack block.
pub(crate) const PS3_PRIMARY_STACK_BASE: u64 = 0xD000_0000;
/// Primary-thread stack size. 64 KB covers the default `SYS_PROCESS_PARAM`
/// stacksize for simple PS3 titles and all CellGov microtests.
pub(crate) const PS3_PRIMARY_STACK_SIZE: usize = 0x0001_0000;
/// Base address of the child-thread stack region. Sits immediately
/// above the primary thread's 64 KB stack, matching the address
/// `ThreadStackAllocator::CHILD_STACK_BASE` hands out. Must be
/// backed by a real guest memory region so child threads can
/// push / pop their stacks.
pub(crate) const PS3_CHILD_STACKS_BASE: u64 = 0xD001_0000;
/// Size of the child-stack region. 15 MB accommodates many
/// simultaneously-live child threads at the PSL1GHT-default 64 KB
/// each; well within the PS3's user-memory footprint.
pub(crate) const PS3_CHILD_STACKS_SIZE: usize = 0x00F0_0000;
/// Highest address reserved 16 bytes below the stack top, matching the
/// PPC64 ABI's requirement for a backchain+linkage area at the frame
/// boundary. `state.gpr[1]` is set to this value on thread entry.
pub(crate) const PS3_PRIMARY_STACK_TOP: u64 =
    PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64 - 0x10;
/// PS3 RSX video/local-memory base (`0xC0000000`). Reserved
/// placeholder; reads return zero, writes fault. Real RSX semantics
/// are out of scope here.
pub(crate) const PS3_RSX_BASE: u64 = 0xC000_0000;
/// RSX reservation size (256 MB) per RPCS3 `vm.cpp`.
pub(crate) const PS3_RSX_SIZE: usize = 0x1000_0000;
/// PS3 SPU-shared / reserved base (`0xE0000000`). Same semantics as
/// the RSX placeholder.
pub(crate) const PS3_SPU_RESERVED_BASE: u64 = 0xE000_0000;
/// SPU reservation size (512 MB) per RPCS3 `vm.cpp`.
pub(crate) const PS3_SPU_RESERVED_SIZE: usize = 0x2000_0000;
