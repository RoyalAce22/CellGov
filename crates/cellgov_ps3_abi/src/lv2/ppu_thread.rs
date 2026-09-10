//! `sys_ppu_thread_*` ABI constants.

/// Smallest `prio` a non-root process may assign to a PPU thread.
/// Priorities run 0 to 3071, with 0 the most urgent.
pub const PPU_THREAD_PRIORITY_MIN: i32 = 0;

/// Smallest `prio` a root or debug process may assign; the kernel
/// widens the range below zero for those processes only.
///
/// This bound is unestablished: no source here fixes where the widened
/// range stops.
pub const PPU_THREAD_PRIORITY_MIN_ROOT: i32 = -512;

/// Largest `prio` any process may assign to a PPU thread, and the
/// least urgent.
pub const PPU_THREAD_PRIORITY_MAX: i32 = 3071;

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
// No public document names this struct. The non-public descriptions
// of the PPU thread surface state the `sys_ppu_thread_create`
// wrapper, which takes the entry point as a plain argument. The
// field order is unestablished. The witness that would fix it is a
// reading of the block that wrapper stages before syscall 52.
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
