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

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/memory_console_tests.rs"]
mod console_tests;
