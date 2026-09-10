//! sys_memory PS3 ABI: user-memory limits.
//!
//! Behaviour (the syscall handlers, the host memory allocator, the
//! free-region tracker) lives in `cellgov_lv2::host::memory`; this
//! module is data only.

/// Total user-memory cap (213 MiB) reported via
/// `sys_memory_get_user_memory_size`.
pub const USER_MEMORY_TOTAL: u32 = 0x0D50_0000;

/// `ipc_key` sentinel meaning "not process-shared" for
/// `sys_mmapper_allocate_shared_memory` (332 / 362); liblv2 passes it
/// on the keyless allocation path.
///
/// The constant's name is inherited vocabulary, not a Sony one.
pub const SYS_MMAPPER_NO_SHM_KEY: u64 = 0xffff_0000_0000_0000;

/// Granule `sys_mmapper_allocate_address` (330) reserves VM areas in,
/// and the only multiple it accepts for `size`. The syscall refuses a
/// `size` that is not a whole number of 256 MiB areas.
pub const VM_AREA_GRANULE: u64 = 0x1000_0000;

/// The `alignment` values `sys_mmapper_allocate_address` accepts:
/// powers of two from the 256 MiB granule up to 0x8000_0000, the
/// largest alignment that fits a 32-bit process address space.
/// Anything else is refused rather than rounded.
pub const VM_AREA_ALIGNMENTS: [u64; 4] = [0x1000_0000, 0x2000_0000, 0x4000_0000, 0x8000_0000];

/// Granule `sys_memory_container_create` (324 / 341) truncates its
/// request to before deciding whether anything is left to allocate. A
/// request under one granule therefore fails for want of memory, not
/// for being small.
pub const CONTAINER_GRANULE: u64 = 0x10_0000;

/// The per-entry attribute table `sys_mmapper_allocate_shared_memory_ext`
/// (339) and its container variant take alongside the key. Only the
/// `type` word carries a known meaning; the rest of the entry is
/// unestablished, and so is the entry length.
pub mod ext_entry {
    /// Byte length of one entry.
    pub const LEN: u32 = 0x18;

    /// Offset of the 64-bit `type` word inside an entry.
    pub const TYPE_OFFSET: u32 = 0x10;

    /// Largest `entry_count` the kernel accepts; zero and negative
    /// counts are refused as well.
    pub const MAX_COUNT: i32 = 0x10;

    /// Entry types accepted without further checks. The membership of
    /// this set is unestablished: nothing here says why 2 is absent.
    pub const PLAIN_TYPES: [u64; 3] = [0, 1, 3];

    /// Entry type that additionally requires 64 KiB pages and a root
    /// or debug process.
    pub const PRIVILEGED_TYPE: u64 = 5;
}

/// `flags` bits selecting the page granule for shared-memory and
/// mmapper-allocated regions. The two page sizes are exclusive: a
/// `flags` word may name one or the other, never both.
pub mod page_size {
    /// Mask over the granularity field the flags below occupy
    /// (`SYS_MEMORY_GRANULARITY_MASK`, bits 8..=11). A `flags` word
    /// whose field holds anything other than zero or one of the flags
    /// below is refused, not rounded.
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
    /// will enforce. A `flags` word that names no page size takes the
    /// same 1 MiB granule as the 1 MiB flag.
    #[must_use]
    pub const fn granule_from_flags(flags: u64) -> u32 {
        if flags & FLAG_64K != 0 {
            GRANULE_64K
        } else {
            GRANULE_1M
        }
    }
}
