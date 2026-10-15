//! `sys_ppu_thread_*` ABI constants.

/// Smallest `prio` a non-root process may assign to a PPU thread.
/// Priorities run 0 to 3071, with 0 the most urgent.
pub const PPU_THREAD_PRIORITY_MIN: i32 = 0;

/// Smallest `prio` a root or debug process may assign; the kernel
/// widens the range below zero for those processes only.
// The console captures of tests/micro/lv2_ppu_thread_answers and
// tests/micro/lv2_ppu_thread_priority_bounds, from a root process,
// show sys_ppu_thread_set_priority taking -512 and refusing -513 and
// -1000 with CELL_EINVAL.
pub const PPU_THREAD_PRIORITY_MIN_ROOT: i32 = -512;

/// Largest `prio` a non-root process may assign to a PPU thread, and
/// the least urgent.
pub const PPU_THREAD_PRIORITY_MAX: i32 = 3071;

/// Largest `prio` a root or debug process may assign with
/// `sys_ppu_thread_set_priority`.
// The console captures of tests/micro/lv2_ppu_thread_priority_bounds
// and tests/micro/lv2_ppu_thread_priority_ceiling, from a root
// process, show every value from 3071 to 3199 taken and 3200 refused
// with CELL_EINVAL. No probe runs as a non-root process, so the
// non-root ceiling stays the documented 3071.
pub const PPU_THREAD_PRIORITY_MAX_ROOT: i32 = 3199;

/// `ppu_thread_param_t`, the block `_sys_ppu_thread_create` (52)
/// reads.
///
/// `entry@0` (u32), `tls@4` (u32), for a declared
/// [`SIZE`](thread_param::SIZE) of 8 bytes.
///
/// `entry` addresses a [`function_descriptor`]. The create
/// dereferences it for the entry point's `code` and `toc`.
///
/// [`function_descriptor`]: crate::format::elf::function_descriptor
// No public document names this struct. The console capture of
// tests/micro/lv2_ppu_thread_answers fixes the field order: a thread
// created from a block with a distinct word at offset 4 starts with
// that word in r13, and runs the entry the word at offset 0 names.
pub mod thread_param {
    /// `sizeof(ppu_thread_param_t)`.
    pub const SIZE: u32 = 0x08;

    /// `entry` -- the guest address of the entry point's descriptor.
    pub const ENTRY_OFFSET: usize = 0x00;
    /// `tls` -- the r13 value the new thread starts under.
    pub const TLS_OFFSET: usize = 0x04;

    // Container coupling, as in `format::elf`: a reader that
    // bounds-checks `SIZE` reads both words without a second check.
    const _: () = assert!(ENTRY_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
    const _: () = assert!(TLS_OFFSET + core::mem::size_of::<u32>() <= SIZE as usize);
}
