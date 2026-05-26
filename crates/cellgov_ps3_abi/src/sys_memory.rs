//! sys_memory PS3 ABI: user-memory limits.
//!
//! Behaviour (the syscall handlers, the host memory allocator, the
//! free-region tracker) lives in `cellgov_lv2::host::memory`; this
//! module is data only.

/// Total user-memory cap (213 MiB) reported via
/// `sys_memory_get_user_memory_size`.
pub const USER_MEMORY_TOTAL: u32 = 0x0D50_0000;

/// `flags` bits selecting the page granule for shared-memory and
/// mmapper-allocated regions. Source:
/// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_memory.h:30-31`.
pub mod page_size {
    /// `SYS_MEMORY_PAGE_SIZE_64K` -- 64 KiB pages.
    pub const FLAG_64K: u64 = 0x200;

    /// `SYS_MEMORY_PAGE_SIZE_1M` -- 1 MiB pages.
    pub const FLAG_1M: u64 = 0x400;

    /// Granule in bytes for the 64 KiB page-size flag.
    pub const GRANULE_64K: u32 = 0x0001_0000;

    /// Granule in bytes for the 1 MiB page-size flag.
    pub const GRANULE_1M: u32 = 0x0010_0000;

    /// Resolve `flags` to the byte granule that `sys_mmapper_map_shared_memory`
    /// will enforce. Matches the `flags & SYS_MEMORY_PAGE_SIZE_64K ?
    /// 0x10000 : 0x100000` branch RPCS3 uses at
    /// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_mmapper.cpp:232`.
    #[must_use]
    pub const fn granule_from_flags(flags: u64) -> u32 {
        if flags & FLAG_64K != 0 {
            GRANULE_64K
        } else {
            GRANULE_1M
        }
    }
}
