//! `sys_ppu_thread_*` ABI constants.

/// Lowest priority a non-root process may assign to a PPU thread.
/// Oracle: RPCS3 `sys_ppu_thread.cpp` `sys_ppu_thread_set_priority`.
pub const PPU_THREAD_PRIORITY_MIN: i32 = 0;

/// Lowest priority a root or debug process may assign; the kernel
/// widens the range below zero for those processes only.
/// Oracle: RPCS3 `sys_ppu_thread.cpp` `sys_ppu_thread_set_priority`.
pub const PPU_THREAD_PRIORITY_MIN_ROOT: i32 = -512;

/// Highest priority any process may assign to a PPU thread.
/// Oracle: RPCS3 `sys_ppu_thread.cpp` `sys_ppu_thread_set_priority`.
pub const PPU_THREAD_PRIORITY_MAX: i32 = 3071;
