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
    mmio_real_address, GAME_OS_MMIO_MAPPINGS, WRITE_HTAB_MMIO_PAGE_SIZE_R3,
};

use crate::group::device_bus::{DeviceBusState, MmioMapping};

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
    let Some(page_shift) = page_shift(firmware, &mapping) else {
        return Some(MmioPage::Refused(
            "a valid entry in an MMIO mapping whose page shift no capture gives",
        ));
    };
    if u64::from(shift) > page_shift {
        return Some(MmioPage::Lv1(WRITE_HTAB_MMIO_PAGE_SIZE_R3));
    }
    let base = mmio_real_address(firmware, mapping.bus, mapping.device, mapping.bus_address);
    Some(match base {
        Some(base) => MmioPage::Io(base + (address - mapping.lpar_address)),
        None => MmioPage::Refused(
            "a valid entry in an MMIO mapping whose real address no lv1 image read gives",
        ),
    })
}

/// The page shift of the call that made `mapping`.
///
/// `map_device_mmio_region` makes a mapping only for a call that a row of
/// [`GAME_OS_MMIO_MAPPINGS`] records, so the row gives the shift.
fn page_shift(firmware: &str, mapping: &MmioMapping) -> Option<u64> {
    GAME_OS_MMIO_MAPPINGS
        .iter()
        .find(|row| {
            row.firmware == firmware
                && row.bus == mapping.bus
                && row.device == mapping.device
                && row.bus_address == mapping.bus_address
                && row.size == mapping.size
                && row.lpar_address == mapping.lpar_address
        })
        .map(|row| row.page_shift)
}
