//! sys_memory PS3 ABI: user-memory limits.
//!
//! Behaviour (the syscall handlers, the host memory allocator, the
//! free-region tracker) lives in `cellgov_lv2::host::memory`; this
//! module is data only.

/// Total user-memory cap (213 MiB) reported via
/// `sys_memory_get_user_memory_size`.
pub const USER_MEMORY_TOTAL: u32 = 0x0D50_0000;

/// `ipc_key` sentinel meaning "not process-shared" for
/// `sys_mmapper_allocate_shared_memory` (332 / 362). Cross-reference:
/// RPCS3's `sys_mmapper.h` (`SYS_MMAPPER_NO_SHM_KEY`, unofficial
/// name); liblv2 passes it on the keyless allocation path.
pub const SYS_MMAPPER_NO_SHM_KEY: u64 = 0xffff_0000_0000_0000;

/// Granule `sys_mmapper_allocate_address` (330) reserves VM areas in,
/// and the only multiple it accepts for `size`. Cross-reference:
/// RPCS3's `sys_mmapper.cpp` `sys_mmapper_allocate_address`.
pub const VM_AREA_GRANULE: u64 = 0x1000_0000;

/// The `alignment` values `sys_mmapper_allocate_address` accepts;
/// anything else is refused rather than rounded. Cross-reference:
/// RPCS3's `sys_mmapper.cpp` `sys_mmapper_allocate_address`.
pub const VM_AREA_ALIGNMENTS: [u64; 4] = [0x1000_0000, 0x2000_0000, 0x4000_0000, 0x8000_0000];

/// Granule `sys_memory_container_create` (324 / 341) truncates its
/// request to before deciding whether anything is left to allocate.
/// Cross-reference: RPCS3's `sys_memory.cpp`
/// `sys_memory_container_create`.
pub const CONTAINER_GRANULE: u64 = 0x10_0000;

/// `flags` bits selecting the page granule for shared-memory and
/// mmapper-allocated regions. Cross-reference: RPCS3's `sys_memory.h`.
pub mod page_size {
    /// Mask over the granularity field the flags below occupy
    /// (`SYS_MEMORY_GRANULARITY_MASK`, bits 8..=11). A `flags` word
    /// whose field holds anything other than the values below is
    /// refused, not rounded. Cross-reference: RPCS3's `sys_memory.h`.
    pub const GRANULARITY_FIELD: u64 = 0xf00;

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
    /// 0x10000 : 0x100000` branch RPCS3 uses in its mmapper handler.
    #[must_use]
    pub const fn granule_from_flags(flags: u64) -> u32 {
        if flags & FLAG_64K != 0 {
            GRANULE_64K
        } else {
            GRANULE_1M
        }
    }
}
