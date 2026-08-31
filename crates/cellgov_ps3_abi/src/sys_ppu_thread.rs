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
