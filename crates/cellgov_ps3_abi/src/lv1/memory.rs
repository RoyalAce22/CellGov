//! LV1 memory regions: the logical partition address that lv1 gives each
//! region it creates in a partition.
//!
//! lv1 keeps one region counter for each partition. A new region takes
//! the counter's value, and lv1 then increments the counter. The region's
//! address holds its size exponent `e` and that counter value:
//!
//! ```text
//! address = (e << 42) | (counter << e)
//! ```
//!
//! [PS3DEVWIKI-HV-RE s:Encoding_LPAR_Memory_Region_Start_Addresses_and_Sizes]
//! gives this encoding and the per-partition counter.

/// The bit position of a region's size exponent in its logical partition
/// address.
///
/// [PS3DEVWIKI-HV-RE s:Encoding_LPAR_Memory_Region_Start_Addresses_and_Sizes] A region address shifted right by 42 is the log2 of the region size.
pub const REGION_SIZE_EXP_SHIFT: u32 = 42;

/// The size exponent of a region of `size` bytes: the least `e` with
/// 2^`e` >= `size`. It is `None` for a size of 0.
///
/// For a size that is not a power of two, the exponent rounds up. The
/// GameOS region of 0xE900000 bytes at 0x700020000000 shows this
/// [PS3DEVWIKI-HV-RE s:GameOS_Physical_Memory_Regions]: its address
/// holds the exponent 28. The 4.93 console capture shows it for both of
/// the 4.93 kernel's boot allocations.
pub const fn region_size_exp(size: u64) -> Option<u32> {
    match size {
        0 => None,
        1 => Some(0),
        _ => Some(u64::BITS - (size - 1).leading_zeros()),
    }
}

/// The logical partition address of a region of `size` bytes that takes
/// the counter value `counter`.
///
/// It is `None` in these cases:
/// - `size` is 0;
/// - `counter << e` does not fit below bit [`REGION_SIZE_EXP_SHIFT`].
pub const fn region_address(size: u64, counter: u64) -> Option<u64> {
    let Some(exp) = region_size_exp(size) else {
        return None;
    };
    if exp >= REGION_SIZE_EXP_SHIFT || counter >> (REGION_SIZE_EXP_SHIFT - exp) != 0 {
        return None;
    }
    Some(((exp as u64) << REGION_SIZE_EXP_SHIFT) | (counter << exp))
}

/// The region counter of the GameOS partition when the kernel makes its
/// first allocation, for each firmware a source gives it for.
///
/// At 4.93 the console capture `lv2_repository_boot_nodes` reads the
/// addresses of the kernel's two boot allocations from its boot record.
/// They hold the counter values 2 and 3. The README of the capture gives
/// the analysis.
pub const GAME_OS_REGION_COUNTER_AT_BOOT: &[(&str, u64)] = &[("4.93", 2)];

/// The GameOS region counter at the kernel's first allocation at the
/// firmware version `firmware`, or `None` when no source gives it.
pub fn game_os_region_counter_at_boot(firmware: &str) -> Option<u64> {
    GAME_OS_REGION_COUNTER_AT_BOOT
        .iter()
        .find(|(fw, _)| *fw == firmware)
        .map(|(_, counter)| *counter)
}

/// The memory unit id that a successful `allocate_memory` returns in r5.
///
/// [PS3DEVWIKI-HV-SYSCALL-REF s:lv1_allocate_memory] Every successful allocation the page reports returned muid 1 in r5.
pub const ALLOCATE_MEMORY_UNIT_ID: u64 = 1;

/// The `allocate_memory` worker in each firmware whose lv1 image shows
/// its argument checks, ascending by firmware.
///
/// The 3.15, 3.41, 3.70 and 4.93 workers hold the same instructions.
/// The 1.94 worker makes the same checks in the same order, with other
/// registers and branch offsets. No 1.50 worker was found. Each
/// `ALLOCATE_MEMORY_*_R3` constant gives the 4.93 address of its check;
/// the 3.15 address is 0x8180 lower.
///
/// [PS3DEVWIKI-HV-RE s:Memory_HV_calls] lv1_allocate_memory is at 0x2D72F0 at 3.15: the wrapper that calls the 3.15 worker.
pub const ALLOCATE_MEMORY_WORKERS: &[(&str, u32)] = &[
    ("1.94", 0x2EEB98),
    ("3.15", 0x2D6CD8),
    ("3.41", 0x2D4D64),
    ("3.70", 0x2DDA90),
    ("4.93", 0x2DEE58),
];

/// Whether the lv1 image of the firmware version `firmware` shows the
/// `allocate_memory` argument checks.
pub fn allocate_memory_checks_read(firmware: &str) -> bool {
    ALLOCATE_MEMORY_WORKERS
        .iter()
        .any(|(fw, _)| *fw == firmware)
}

/// The largest size, in r3, that passes the size check of
/// `allocate_memory`. The worker builds it with `li r0, -1` and
/// `clrldi r0, r0, 2`.
pub const ALLOCATE_MEMORY_MAX_SIZE: u64 = 0x3FFF_FFFF_FFFF_FFFF;

/// The smallest page size exponent, in r4, that passes the check of
/// `allocate_memory`.
pub const ALLOCATE_MEMORY_PAGE_SIZE_EXP_MIN: u64 = 12;

/// The largest page size exponent, in r4, that passes the check of
/// `allocate_memory`. The worker compares r4 - 12 with 49, unsigned.
pub const ALLOCATE_MEMORY_PAGE_SIZE_EXP_MAX: u64 = 61;

/// The largest value of r5 that passes the check of `allocate_memory`.
pub const ALLOCATE_MEMORY_R5_MAX: u64 = 1;

/// The flag bits, in r6, that pass the check of `allocate_memory`: the
/// low 4.
pub const ALLOCATE_MEMORY_FLAGS_MASK: u64 = 0xF;

/// The r3 of `allocate_memory` for a size above
/// [`ALLOCATE_MEMORY_MAX_SIZE`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17) in
/// the low word.
///
/// The worker marks the failing argument in the high word of r3:
/// - bit 32 is r3, the size;
/// - bit 33 is r4, the page size exponent;
/// - bit 34 is r5;
/// - bit 35 is r6, the flags.
///
/// The worker checks the arguments in this order, and the first failure
/// is the answer:
/// 1. the size, at [OFW-4.93 lv1 a:0x2DEF60];
/// 2. the page size exponent, at [OFW-4.93 lv1 a:0x2DEF9C];
/// 3. the size's alignment to the page size, at
///    [OFW-4.93 lv1 a:0x2DEFB4];
/// 4. r5, at [OFW-4.93 lv1 a:0x2DEF30];
/// 5. the flags, at [OFW-4.93 lv1 a:0x2DF014].
///
/// The partition check before them, at [OFW-4.93 lv1 a:0x2DEEC4], reads
/// lv1's own state and no argument.
pub const ALLOCATE_MEMORY_SIZE_TOO_LARGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 of `allocate_memory` for a page size exponent outside
/// [`ALLOCATE_MEMORY_PAGE_SIZE_EXP_MIN`] to
/// [`ALLOCATE_MEMORY_PAGE_SIZE_EXP_MAX`]: -17, with r4 marked. The check
/// is at [OFW-4.93 lv1 a:0x2DEF9C].
pub const ALLOCATE_MEMORY_PAGE_SIZE_EXP_R3: u64 = 0x0000_0002_FFFF_FFEF;

/// The r3 of `allocate_memory` for a size that is not a multiple of the
/// page size: -17, with r3 and r4 marked. The check is at
/// [OFW-4.93 lv1 a:0x2DEFB4]. A size of 0 passes it.
pub const ALLOCATE_MEMORY_SIZE_ALIGNMENT_R3: u64 = 0x0000_0003_FFFF_FFEF;

/// The r3 of `allocate_memory` for an r5 above
/// [`ALLOCATE_MEMORY_R5_MAX`]: `LV1_NO_ENTRY` (-6), with r5 marked. The
/// check is at [OFW-4.93 lv1 a:0x2DEF30].
pub const ALLOCATE_MEMORY_R5_R3: u64 = 0x0000_0004_FFFF_FFFA;

/// The r3 of `allocate_memory` for flags outside
/// [`ALLOCATE_MEMORY_FLAGS_MASK`]: -17, with r6 marked. The check is at
/// [OFW-4.93 lv1 a:0x2DF014].
pub const ALLOCATE_MEMORY_FLAGS_R3: u64 = 0x0000_0008_FFFF_FFEF;

/// The r3 of `write_htab_entry` for a valid entry whose page lies in a
/// region that `allocate_memory` created and is larger than the region's
/// pages: `LV1_ACCESS_VIOLATION` (-5), with r6 marked.
///
/// After its argument checks, the 4.93 `allocate_memory` worker does
/// these steps:
/// 1. It keeps the size, the page size exponent and an array of page
///    addresses in an 80-byte object
///    [OFW-4.93 lv1 a:0x2DF05C-0x2DF0D4].
/// 2. It gives each page of the region a page from lv1's page allocator
///    [OFW-4.93 lv1 a:0x2DF0FC-0x2DF12C]. The allocator fills the page
///    with zeros [OFW-4.93 lv1 a:0x2C14F8-0x2C1508].
/// 3. It constructs the region [OFW-4.93 lv1 a:0x2DF244]. The region
///    keeps its address at +80 [OFW-4.93 lv1 a:0x2CF4C8], the page size
///    exponent at +160 [OFW-4.93 lv1 a:0x2CF4F8] and the object at +176
///    [OFW-4.93 lv1 a:0x2DEAE0-0x2DEAE8].
/// 4. The region's class is in the TOC slot at r2 - 8104
///    [OFW-4.93 lv1 a:0x2DEAC4-0x2DEACC] [OFW-4.93 lv1 a:0x358090].
///
/// `write_htab_entry` calls the method at +24 of that class
/// [OFW-4.93 lv1 a:0x3621E8] [OFW-4.93 lv1 a:0x347BC8] with the page's
/// offset in the region. The method does these steps in this order:
/// 1. It checks that the region is live and is the caller's
///    [OFW-4.93 lv1 a:0x2DDFC0-0x2DE000] [OFW-4.93 lv1 a:0x2DDE54-0x2DDE80].
///    A region that the caller's `allocate_memory` created passes.
/// 2. A page shift above the exponent at +160 returns the code
///    0x997997D2B58E3C02 [OFW-4.93 lv1 a:0x2DE008-0x2DE01C]
///    [OFW-4.93 lv1 a:0x2DE080]. The handler gives this r3 for that code
///    [OFW-4.93 lv1 a:0x2ED5C4-0x2ED5DC] [OFW-4.93 lv1 a:0x2ED60C-0x2ED618].
/// 3. The real address is the address of the page that holds the offset,
///    plus the offset in that page [OFW-4.93 lv1 a:0x2DE020-0x2DE050].
///
/// The method does not read the writer's read only flag. The capture
/// `lv1_allocated_page_translation` reads the cited code, slot, class
/// entry and descriptor in the reference console's lv1.
pub const WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3: u64 = 0x0000_0008_FFFF_FFFB;

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/memory_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/memory_check_tests.rs"]
mod check_tests;

#[cfg(test)]
#[path = "tests/memory_allocated_page_console_tests.rs"]
mod allocated_page_console_tests;
