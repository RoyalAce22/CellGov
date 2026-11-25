//! LV1 virtual address spaces:
//! - the argument checks and outputs of `construct_virtual_address_space`
//!   (hypercall 2);
//! - the outcomes of `select_virtual_address_space` (hypercall 7);
//! - the vas id of the 4.93 kernel's boot address space.
//!
//! The checks and outputs come from the 4.93 `lv1.self`, decrypted
//! offline and read with `cellgov dev disasm`. The vas id comes from the
//! console capture [`VAS_BOOT_RECORD_CAPTURE`]. The repository holds no
//! byte of lv1, only these facts.
//!
//! A handler puts its outputs in the vector's return slots, and the
//! vector loads r4 to r10 from them [OFW-4.93 lv1 a:0x1044-0x1068]. The
//! vector's entry zeroes the slots, so each output that a handler does
//! not write returns 0.
//!
//! The select handler gives these outcomes:
//! - A vas id of 0 selects the partition's default address space and
//!   returns 0 [OFW-4.93 lv1 a:0x2EB078] [OFW-4.93 lv1 a:0x2EB258].
//! - Another id that names no address space of the partition returns
//!   `LV1_NO_ENTRY` (-6) with r3 marked in the high word, 0x1_FFFFFFFA
//!   [OFW-4.93 lv1 a:0x2EB198]. The handler looks the id up in the partition's object
//!   table and checks the object's class and its state word at +144.
//! - An id that names an address space that the partition constructed
//!   selects that address space and returns 0.
//!
//! The handler sets the address space of the PPE object that the calling
//! thread's lv1 context names [OFW-4.93 lv1 a:0x2EB26C].

/// The handlers of hypercalls 2 and 7 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressSpaceHandlers {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `construct_virtual_address_space` handler.
    pub construct: u32,
    /// The address of the `select_virtual_address_space` handler.
    pub select: u32,
}

/// Each firmware whose lv1 image shows the two handlers, ascending.
///
/// The addresses are the entries of hypercalls 2 and 7 in the live 4.93
/// hypercall table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const ADDRESS_SPACE_HANDLERS: &[AddressSpaceHandlers] = &[AddressSpaceHandlers {
    firmware: "4.93",
    construct: 0x2EC554,
    select: 0x2EAFE4,
}];

/// The handlers of the firmware version `firmware`, or `None` when no
/// lv1 image read shows them.
pub fn address_space_handlers(firmware: &str) -> Option<&'static AddressSpaceHandlers> {
    ADDRESS_SPACE_HANDLERS
        .iter()
        .find(|handlers| handlers.firmware == firmware)
}

/// The largest size exponent of the page table, in r3, that passes the
/// check of `construct_virtual_address_space`. The handler compares
/// r3 with 20, unsigned, at [OFW-4.93 lv1 a:0x2EC5F8].
pub const HTAB_SIZE_EXP_MAX: u64 = 20;

/// The smallest size exponent of the page table that lv1 uses. The handler
/// raises an r3 below 18 to 18 at [OFW-4.93 lv1 a:0x2EC64C], before any other use of r3.
pub const HTAB_SIZE_EXP_MIN: u64 = 18;

/// The size exponent of the page table that lv1 uses for an r3 of `r3`,
/// or `None` when r3 fails the check against [`HTAB_SIZE_EXP_MAX`].
pub const fn htab_size_exp(r3: u64) -> Option<u64> {
    if r3 > HTAB_SIZE_EXP_MAX {
        None
    } else if r3 < HTAB_SIZE_EXP_MIN {
        Some(HTAB_SIZE_EXP_MIN)
    } else {
        Some(r3)
    }
}

/// The page table size, in bytes, that `construct_virtual_address_space`
/// returns in r5 for the exponent `exp`: 2^`exp`.
///
/// The constructor at [OFW-4.93 lv1 a:0x2D1FDC] stores the exponent at +116 of the new
/// address space [OFW-4.93 lv1 a:0x2D2050]. The handler loads it and shifts 1 left by
/// it [OFW-4.93 lv1 a:0x2EC6C0].
pub const fn htab_size(exp: u64) -> u64 {
    1 << exp
}

/// The most page sizes, in r4, that `construct_virtual_address_space`
/// checks past. The function at [OFW-4.93 lv1 a:0x2B9CE0] returns it, and the handler
/// compares it with r4, unsigned, at [OFW-4.93 lv1 a:0x2EC60C].
pub const PAGE_SIZE_COUNT_MAX: u64 = 2;

/// The page shifts that `construct_virtual_address_space` accepts.
///
/// The function at [OFW-4.93 lv1 a:0x2B9CF8] checks the top r4 bytes of r5, from the most
/// significant byte. It accepts a count of 0, and it ignores the bytes
/// past the count.
pub const PAGE_SHIFTS: &[u64] = &[16, 20, 24];

/// Whether the first `count` page shifts of `shifts` are each one of
/// [`PAGE_SHIFTS`]. A count above [`PAGE_SIZE_COUNT_MAX`] fails, as the
/// function at [OFW-4.93 lv1 a:0x2B9CF8] refuses a count above 2 [OFW-4.93 lv1 a:0x2B9CFC].
pub fn page_shifts_accepted(count: u64, shifts: u64) -> bool {
    count <= PAGE_SIZE_COUNT_MAX
        && (0..count)
            .map(|index| (shifts >> (56 - 8 * index)) & 0xFF)
            .all(|shift| PAGE_SHIFTS.contains(&shift))
}

/// The r3 of `construct_virtual_address_space` for an r3 above
/// [`HTAB_SIZE_EXP_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x2EC7A4].
///
/// As with `allocate_memory`, bit 32 marks r3, bit 33 r4 and bit 34 r5.
/// The handler checks the arguments in this order, and the first failure
/// is the answer:
/// 1. the size exponent of the page table;
/// 2. the page size count, at [OFW-4.93 lv1 a:0x2EC610];
/// 3. the page shifts, at [OFW-4.93 lv1 a:0x2EC63C].
///
/// The handler makes one more check between 2 and 3: a count above
/// 0x7FFFFFFF gets the r5 mark [OFW-4.93 lv1 a:0x2EC620]. No count that passes check 2
/// fails it.
///
/// Past the checks, the call returns `LV1_RESOURCE_SHORTAGE` (-2),
/// zero-extended, when lv1 cannot allocate the address space or its page
/// table [OFW-4.93 lv1 a:0x2EC780].
pub const CONSTRUCT_VAS_HTAB_SIZE_EXP_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 of `construct_virtual_address_space` for a page size count
/// above [`PAGE_SIZE_COUNT_MAX`]: -17, with r4 marked [OFW-4.93 lv1 a:0x2EC7B8].
pub const CONSTRUCT_VAS_PAGE_SIZE_COUNT_R3: u64 = 0x0000_0002_FFFF_FFEF;

/// The r3 of `construct_virtual_address_space` for a page shift that is
/// not one of [`PAGE_SHIFTS`]: -17, with r5 marked [OFW-4.93 lv1 a:0x2EC790].
pub const CONSTRUCT_VAS_PAGE_SHIFT_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The console capture of the 4.93 kernel's record of its virtual
/// address space, relative to the workspace root.
/// Its README gives the method and the analysis.
pub const VAS_BOOT_RECORD_CAPTURE: &str = "tests/micro/lv2_vas_boot_record/ps3/cech20-cex-493";

/// The vas id that lv1 gives the GameOS kernel's boot address space, for
/// each firmware a source gives it for.
///
/// lv1 takes the id from the partition's object table at [OFW-4.93 lv1 a:0x2CC6EC]. The
/// table's other objects decide the id, so no image read gives it. At
/// 4.93 the capture [`VAS_BOOT_RECORD_CAPTURE`] reads it from the
/// kernel's record.
pub const GAME_OS_BOOT_VAS_ID: &[(&str, u64)] = &[("4.93", 2)];

/// The vas id of the GameOS kernel's boot address space at the firmware
/// version `firmware`, or `None` when no source gives it.
pub fn game_os_boot_vas_id(firmware: &str) -> Option<u64> {
    GAME_OS_BOOT_VAS_ID
        .iter()
        .find(|(fw, _)| *fw == firmware)
        .map(|(_, id)| *id)
}

#[cfg(test)]
#[path = "tests/address_space_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/address_space_console_tests.rs"]
mod console_tests;
