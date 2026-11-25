//! Hypercall 173, `unmap_device_mmio_region`: lv1 removes an MMIO mapping
//! that `map_device_mmio_region` made for the calling partition.
//!
//! The handler answers the call for devices 0, 1 and 2 of bus 1, as far
//! as a source gives lv1's path; see
//! [`cellgov_ps3_abi::lv1::device_bus`].

use cellgov_mem::be::read_u64;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_BYTES, PTE_RPN};
use cellgov_ps3_abi::lv1::address_space::htab_size;
use cellgov_ps3_abi::lv1::call::UNMAP_DEVICE_MMIO_REGION;
use cellgov_ps3_abi::lv1::device_bus::{
    mmio_real_address, mmio_regions_read, mmio_unmap_handler, open_device_path_read,
    MmioRealAddress, MMIO_UNMAP_BUS_RANGE_R3, MMIO_UNMAP_DEVICE_0_R3, MMIO_UNMAP_NOT_OPEN_R3,
    MMIO_UNMAP_NO_BUS_R3, MMIO_UNMAP_NO_MAPPING_R3, OPEN_DEVICE_BUS_MAX, OPEN_DEVICE_READ_BUS,
};
use cellgov_ps3_abi::lv1::page_table::valid;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{DeviceBusChange, DeviceBusState, MmioMapping, OpenDevice};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::group::memory::address_space::AddressSpaceState;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: UNMAP_DEVICE_MMIO_REGION,
    name: "unmap_device_mmio_region",
    cost: None,
    handler: |request, input| {
        unmap_mmio(
            request,
            input.firmware,
            input.groups.device_bus(),
            input.groups.memory().spaces(),
            input.storage,
        )
    },
}];

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("an unmap_device_mmio_region at a firmware whose lv1 handler is not read");

/// The call for bus 2, 3 or 4.
const OTHER_BUS: UnmodelledError = UnmodelledError::new(
    "an unmap_device_mmio_region for bus 2, 3 or 4, whose path through lv1's unmap function is \
     not read in full",
);

/// The call for a device of bus 1 above device 2.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "an unmap_device_mmio_region for a device of bus 1 other than devices 0, 1 and 2, whose lv1 \
     lookup is not read",
);

/// The call for an open device whose region slots no source gives.
const UNREAD_REGIONS: UnmodelledError = UnmodelledError::new(
    "an unmap_device_mmio_region for an open device of bus 1 whose MMIO region slots no capture \
     reads",
);

/// The call at an address inside a mapping, past its first byte.
const INSIDE_MAPPING: UnmodelledError = UnmodelledError::new(
    "an unmap_device_mmio_region at an address inside an MMIO mapping other than its start, whose \
     free in lv1's region set is not read",
);

/// The call for a mapping whose region a page table entry maps.
const MAPPED_PAGE: UnmodelledError = UnmodelledError::new(
    "an unmap_device_mmio_region while a page table entry maps a page of the mapping's MMIO, \
     whose release calls methods of the region's users that are not read",
);

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 173: removes the MMIO mapping at the logical partition
/// address r5 of device r4 of bus r3.
///
/// A call that fails one of lv1's checks gets lv1's own r3 and r4 to r10
/// cleared. See [`cellgov_ps3_abi::lv1::device_bus`] for the checks and
/// their order. A call at the start of a mapping of the device gets 0 in
/// r3 and r4 to r10 cleared. The answer carries the removal of the
/// mapping.
///
/// lv1's free of the address leaves the region counter as it is; see
/// [`map_mmio`](super::mmio_map::map_mmio) for the reads.
///
/// The model makes these assumptions:
/// - lv1's mappings in the device's regions hold a partition other than
///   the caller's.
/// - The doubleword at lv1's address 72 holds 0 at run time, as lv1's
///   image holds.
/// - The free finds the address of a mapping that the map made, and its
///   wait for the region's users ends.
/// - The release of the region gives 0.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 is bus 2, 3 or 4;
/// - r4 names a device of bus 1 above device 2;
/// - the device is open, and no capture reads its region slots;
/// - r5 lies in a mapping of the device past the mapping's start;
/// - a valid entry of a page table in `storage` maps a page of the
///   mapping's MMIO; see [`maps_mmio`].
pub(crate) fn unmap_mmio(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &DeviceBusState,
    spaces: &AddressSpaceState,
    storage: &GuestMemory,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| mmio_unmap_handler(fw).is_some()) else {
        return UNREAD_FIRMWARE.refusal(request);
    };
    let (bus, device, address) = (request.r(3), request.r(4), request.r(5));
    if bus > OPEN_DEVICE_BUS_MAX {
        return lv1_answer(MMIO_UNMAP_BUS_RANGE_R3);
    }
    if bus == 0 {
        return lv1_answer(MMIO_UNMAP_NO_BUS_R3);
    }
    if bus != OPEN_DEVICE_READ_BUS {
        return OTHER_BUS.refusal(request);
    }
    if !open_device_path_read(bus, device) {
        return OTHER_DEVICE.refusal(request);
    }
    if !state.is_open(OpenDevice { bus, device }) {
        return lv1_answer(MMIO_UNMAP_NOT_OPEN_R3);
    }
    if device == 0 {
        return lv1_answer(MMIO_UNMAP_DEVICE_0_R3);
    }
    if !mmio_regions_read(firmware, bus, device) {
        return UNREAD_REGIONS.refusal(request);
    }
    let Some(mapping) = state.mmio_mappings().find(|mapping| {
        mapping.bus == bus
            && mapping.device == device
            && address >= mapping.lpar_address
            && address - mapping.lpar_address < mapping.size
    }) else {
        return lv1_answer(MMIO_UNMAP_NO_MAPPING_R3);
    };
    if address != mapping.lpar_address {
        return INSIDE_MAPPING.refusal(request);
    }
    if maps_mmio(firmware, &mapping, spaces, storage) {
        return MAPPED_PAGE.refusal(request);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(DeviceBusChange::MmioRegionUnmapped(mapping));
    HypercallDisposition::Answered(answer)
}

/// Whether a valid entry of a page table in `spaces` maps a real page in
/// the MMIO of `mapping`.
///
/// The test reads the real page of each valid entry. Two mappings of one
/// region share their real pages. So an entry that another mapping's
/// page gave also counts.
///
/// A table with no storage holds no entry, because `write_htab_entry`
/// writes no entry to it. A mapping whose real address no source gives
/// has no entry, because `write_htab_entry` refuses its pages.
fn maps_mmio(
    firmware: &str,
    mapping: &MmioMapping,
    spaces: &AddressSpaceState,
    storage: &GuestMemory,
) -> bool {
    let MmioRealAddress::Real(base) =
        mmio_real_address(firmware, mapping.bus, mapping.device, mapping.bus_address)
    else {
        return false;
    };
    spaces.spaces().iter().any(|space| {
        let bytes = spaces
            .table_real(space.vas_id)
            .and_then(|table| ByteRange::new(GuestAddr::new(table), htab_size(space.htab_size_exp)))
            .and_then(|range| storage.read(range));
        bytes.is_some_and(|bytes| {
            bytes.chunks_exact(PTE_BYTES).any(|entry| {
                let real = read_u64(entry, 8) & PTE_RPN;
                valid(read_u64(entry, 0)) && real >= base && real - base < mapping.size
            })
        })
    })
}

#[cfg(test)]
#[path = "tests/mmio_unmap_tests.rs"]
mod tests;
