//! sys_memory PS3 ABI: user-memory limits.
//!
//! Behaviour (the syscall handlers, the host memory allocator, the
//! free-region tracker) lives in `cellgov_lv2::host::memory`; this
//! module is data only.

/// Total user-memory cap (213 MiB) reported via
/// `sys_memory_get_user_memory_size`.
pub const USER_MEMORY_TOTAL: u32 = 0x0D50_0000;
