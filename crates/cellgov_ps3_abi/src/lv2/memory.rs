//! sys_memory PS3 ABI: user-memory limits.
//!
//! Behaviour (the syscall handlers, the host memory allocator, the
//! free-region tracker) lives in `cellgov_lv2::host::memory`; this
//! module is data only.

/// The user memory a game process holds (209 MiB) before it creates a
/// memory container: the `total` `sys_memory_get_user_memory_size`
/// reports while no container is live.
// The lv2_memory_free_container, lv2_user_memory_stack_1m and
// lv2_user_memory_big_image captures read this total for a 64 KiB and a
// 1 MiB primary stack and for an image 8 MiB larger: neither input
// moves it.
pub const USER_MEMORY_TOTAL: u32 = 0x0D10_0000;

/// `ipc_key` sentinel meaning "not process-shared" for
/// `sys_mmapper_allocate_shared_memory` (332 / 362); liblv2 passes it
/// on the keyless allocation path.
///
/// The constant's name is inherited vocabulary, not a Sony one.
// The console captures of tests/micro/lv2_mmapper_allocate and
// tests/micro/lv2_memory_constants grant a game process segment after
// segment under it, where a game's 332 under key 0, which no one holds,
// answers ESRCH.
pub const SYS_MMAPPER_NO_SHM_KEY: u64 = 0xffff_0000_0000_0000;

/// Granule `sys_mmapper_allocate_address` (330) reserves VM areas in,
/// and the only multiple it accepts for `size`. The syscall refuses a
/// `size` that is not a whole number of 256 MiB areas.
// The console capture of tests/micro/lv2_memory_constants answers
// EALIGN for 0x0FFFFFFF and 0x18000000 and grants 0x20000000; the
// lv2_mmapper_allocate capture answers EALIGN for 0x1000.
pub const VM_AREA_GRANULE: u64 = 0x1000_0000;

/// The `alignment` values `sys_mmapper_allocate_address` accepts:
/// powers of two from the 256 MiB granule up to 0x8000_0000, the
/// largest alignment that fits a 32-bit process address space. The
/// area's base is a multiple of the alignment. Zero takes the granule;
/// anything else is refused rather than rounded.
// The console capture of tests/micro/lv2_memory_constants grants each
// of the four, the first area at 0x50000000, 0x60000000, 0x80000000
// and 0x80000000, and answers EALIGN for 0x08000000 and 0x30000000.
// The lv2_mmapper_allocate capture grants alignment 0 and answers
// EALIGN for 0x3000.
pub const VM_AREA_ALIGNMENTS: [u64; 4] = [0x1000_0000, 0x2000_0000, 0x4000_0000, 0x8000_0000];

/// Granule `sys_memory_container_create` (324 / 341) truncates its
/// request to before deciding whether anything is left to allocate. A
/// request under one granule therefore fails for want of memory, not
/// for being small.
// The console capture of tests/micro/lv2_memory_constants answers
// ENOMEM for 0xFFFFF, and a container of 0x1FFFFF takes 1 MiB of the
// available memory, as one of 0x100000 does.
pub const CONTAINER_GRANULE: u64 = 0x10_0000;

/// The per-entry attribute table `sys_mmapper_allocate_shared_memory_ext`
/// (339) and its container variant take alongside the key. Only the
/// `type` word carries a known meaning; no capture reads the rest of an
/// entry.
// The console captures of tests/micro/lv2_mmapper_allocate and
// tests/micro/lv2_memory_constants, from a game process.
pub mod ext_entry {
    /// Byte length of one entry.
    // lv2_memory_constants: in a two-entry table, type 2 at byte 0x28
    // is refused and at byte 0x30 is not, and a one-entry table ending
    // at the last mapped byte is read whole.
    pub const LEN: u32 = 0x18;

    /// Offset of the 64-bit `type` word inside an entry.
    pub const TYPE_OFFSET: u32 = 0x10;

    /// Largest `entry_count` the kernel accepts; zero and negative
    /// counts are refused as well.
    // lv2_mmapper_allocate: 16 granted, 0 and 17 EINVAL;
    // lv2_memory_constants: -1 EINVAL.
    pub const MAX_COUNT: i32 = 0x10;

    /// Entry types a game process may name. Types 2 and 4 to 8 answer
    /// EPERM.
    // lv2_mmapper_allocate sweeps 0 to 6, lv2_memory_constants 7 and 8.
    pub const PLAIN_TYPES: [u64; 3] = [0, 1, 3];

    /// Entry type CellGov admits for a root or debug process with
    /// 64 KiB pages. A game process's type 5 with 64 KiB pages answers
    /// EPERM on the console (lv2_mmapper_allocate); no capture shows it
    /// granted.
    pub const PRIVILEGED_TYPE: u64 = 5;
}

/// `flags` bits selecting the page granule for shared-memory and
/// mmapper-allocated regions. The two page sizes are exclusive: a
/// `flags` word may name one or the other, never both.
// The console capture of tests/micro/lv2_memory_constants allocates
// 1 MiB with sys_memory_allocate (348) under flags 0, 0x200 and 0x400,
// reading 1 MiB, 64 KiB and 1 MiB pages back, and answers EINVAL for
// 0x100, 0x600, 0x800 and 0xF00.
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

    /// The granule `flags` selects, `None` when its granularity field
    /// holds anything other than zero, [`FLAG_64K`] or [`FLAG_1M`].
    #[must_use]
    pub const fn accepted_granule(flags: u64) -> Option<u32> {
        match flags & GRANULARITY_FIELD {
            0 | FLAG_64K | FLAG_1M => Some(granule_from_flags(flags)),
            _ => None,
        }
    }
}
