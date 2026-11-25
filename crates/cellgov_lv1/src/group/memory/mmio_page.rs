//! The answer of `write_htab_entry` for a page in an MMIO mapping that
//! `map_device_mmio_region` made.
//!
//! lv1 makes a memory region of the partition for each MMIO mapping. It
//! translates a page of that region to the device's MMIO, at a real
//! address above [`STORED_MEMORY_LIMIT`]; see
//! [`WRITE_HTAB_MMIO_PAGE_SIZE_R3`]. CellGov holds no storage for that
//! address.
//!
//! [`STORED_MEMORY_LIMIT`]: cellgov_ps3_abi::lv1::page_table::STORED_MEMORY_LIMIT

use cellgov_ps3_abi::lv1::device_bus::{
    mmio_real_address, MmioRealAddress, WRITE_HTAB_MMIO_PAGE_SIZE_R3,
};

use crate::group::device_bus::DeviceBusState;

/// The answer of `write_htab_entry` for a valid entry whose page lies in
/// an MMIO mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MmioPage {
    /// The page lies in the device's MMIO, at this real address.
    Io(u64),
    /// lv1's translation refuses the entry with this r3.
    Lv1(u64),
    /// CellGov refuses the entry for this reason.
    Refused(&'static str),
}

/// The answer for a valid entry whose page of 2^`shift` bytes lies at the
/// logical partition address `address`, at the firmware `firmware`.
///
/// The answer is `None` when no MMIO mapping in `device_bus` holds the
/// address. lv1 finds the mapping's region by the page's address alone.
pub(crate) fn mapping_page(
    firmware: &str,
    device_bus: &DeviceBusState,
    address: u64,
    shift: u32,
) -> Option<MmioPage> {
    let mapping = device_bus
        .mmio_mappings()
        .find(|m| address >= m.lpar_address && address - m.lpar_address < m.size)?;
    if u64::from(shift) > mapping.page_shift {
        return Some(MmioPage::Lv1(WRITE_HTAB_MMIO_PAGE_SIZE_R3));
    }
    let base = mmio_real_address(firmware, mapping.bus, mapping.device, mapping.bus_address);
    Some(match base {
        MmioRealAddress::Real(base) => MmioPage::Io(base + (address - mapping.lpar_address)),
        MmioRealAddress::NoRegion => MmioPage::Refused(
            "a valid entry in an MMIO mapping whose region no lv1 image read gives",
        ),
        MmioRealAddress::NoIndex => MmioPage::Refused(
            "a valid entry in an MMIO mapping of a device whose index no lv1 image read gives",
        ),
        MmioRealAddress::BusDoubleword => MmioPage::Refused(
            "a valid entry in an MMIO mapping of a device of index 6, \
             whose real address adds a bus object doubleword that CellGov does not hold",
        ),
    })
}
