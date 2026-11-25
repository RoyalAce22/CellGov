//! The memory region checks that `read_virtual_uart` and
//! `write_virtual_uart` make on the buffer of a call.
//!
//! The two handlers look up the region of r4 with one function, and check
//! its class, its bytes from r4 and its page array in one order; see
//! [`cellgov_ps3_abi::lv1::uart`].

use cellgov_ps3_abi::lv1::interrupt::region_lookup_base;

use crate::group::memory::region::{MemoryRegion, MemoryState};

/// The memory region that holds a buffer that passes the checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BufferRegion {
    /// The real mode region, whose base is 0.
    RealMode,
    /// This region, which `allocate_memory` created.
    Allocated(MemoryRegion),
}

/// The outcome of the region checks on a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BufferCheck {
    /// The buffer passes each check.
    Held(BufferRegion),
    /// No region holds the buffer. lv1 returns its r3 for no region.
    NoRegion,
    /// The region has fewer bytes from the buffer to its end than the call
    /// gives. lv1 returns its r3 for a region check.
    TooFewBytes,
    /// The buffer is in the real mode region, and no source gives the real
    /// mode size for the firmware.
    RealModeSizeUnknown,
    /// The buffer is in a region that CellGov does not hold.
    RegionNotHeld,
}

/// The region checks on the `size` bytes at the logical partition address `address`.
///
/// The caller refuses an address above the handler's bound before it calls
/// this function. CellGov makes the checks for these regions:
/// - the real mode region, whose base is 0. lv1 checks its class and its
///   page array as it does for `configure_virtual_uart_irq`, whose kernel
///   call passes them on the console.
/// - each region that `allocate_memory` created. Each passes the class
///   check and the page array check; see [`allocated_region`].
pub(super) fn check(
    firmware: &str,
    regions: &[MemoryRegion],
    address: u64,
    size: u64,
    real_mode_size: fn(&str) -> Option<u64>,
) -> BufferCheck {
    let base = region_lookup_base(address);
    let (region, region_size) = if base == 0 {
        let Some(rm_size) = real_mode_size(firmware) else {
            return BufferCheck::RealModeSizeUnknown;
        };
        (BufferRegion::RealMode, rm_size)
    } else {
        let Some(region) = allocated_region(regions, base) else {
            return BufferCheck::RegionNotHeld;
        };
        (BufferRegion::Allocated(*region), region.size)
    };
    // lv1's lookup finds no region when the base plus the size is not
    // above r4 [OFW-4.93 lv1 a:0x2CE798-0x2CE7A8].
    let offset = address - base;
    if offset >= region_size {
        return BufferCheck::NoRegion;
    }
    if region_size - offset < size {
        return BufferCheck::TooFewBytes;
    }
    BufferCheck::Held(region)
}

/// The real address in CellGov's storage of the logical partition address
/// `address` in `region`, or `None` when the storage does not hold the
/// region.
///
/// The real mode region lies at real address 0 of the storage. Each
/// allocated region with storage lies at the real address that the memory
/// group gives it.
pub(super) fn real_address(
    memory: &MemoryState,
    region: BufferRegion,
    address: u64,
) -> Option<u64> {
    match region {
        BufferRegion::RealMode => Some(address),
        BufferRegion::Allocated(region) => {
            let storage = memory.storage_of(&region)?;
            storage.checked_add(address.checked_sub(region.lpar_address)?)
        }
    }
}

/// The region that `allocate_memory` created at `base`.
///
/// Such a region passes the class check and the page array check of both
/// handlers:
/// - The constructor that the worker calls [OFW-4.93 lv1 a:0x2DF244]
///   stores the word of the TOC slot at r2 - 8096 at +72
///   [OFW-4.93 lv1 a:0x2DEA48] [OFW-4.93 lv1 a:0x2DEAAC]
///   [OFW-4.93 lv1 a:0x2CF474] [OFW-4.93 lv1 a:0x2CF4C4]. That word
///   [OFW-4.93 lv1 a:0x358098] is the word of the slot at r2 - 5872 that
///   the handlers check [OFW-4.93 lv1 a:0x358948].
/// - The constructor stores the region's page object at +176
///   [OFW-4.93 lv1 a:0x2DEAE0-0x2DEAE8].
fn allocated_region(regions: &[MemoryRegion], base: u64) -> Option<&MemoryRegion> {
    regions.iter().find(|region| region.lpar_address == base)
}
