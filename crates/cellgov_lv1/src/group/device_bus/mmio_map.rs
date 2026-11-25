//! Hypercall 172, `map_device_mmio_region`: lv1 maps an MMIO region of a
//! device at a logical partition address of the calling partition.
//!
//! The handler answers the call for the devices of bus 1 that the
//! partition holds open, as far as a source gives lv1's path; see
//! [`cellgov_ps3_abi::lv1::device_bus`]. The state holds each mapping
//! that lv1 made for the partition.

use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::MAP_DEVICE_MMIO_REGION;
use cellgov_ps3_abi::lv1::device_bus::{
    game_os_mmio_mapping, mmio_map_handler, mmio_region, mmio_regions_read, open_device_path_read,
    MMIO_MAP_BUS_RANGE_R3, MMIO_MAP_DEVICE_0_R3, MMIO_MAP_NOT_OPEN_R3, MMIO_MAP_NO_BUS_R3,
    MMIO_MAP_NO_REGION_R3, MMIO_MAP_PAGE_SHIFTS, MMIO_MAP_PAGE_SHIFT_R3, MMIO_MAP_SIZE_R3,
    OPEN_DEVICE_BUS_MAX, OPEN_DEVICE_READ_BUS,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{DeviceBusChange, DeviceBusState, OpenDevice};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: MAP_DEVICE_MMIO_REGION,
    name: "map_device_mmio_region",
    cost: None,
    handler: |request, input| map_mmio(request, input.firmware, input.groups.device_bus()),
}];

/// One mapping that lv1 made in an MMIO region of a device for the
/// partition.
///
/// lv1 keeps the partition, the logical partition address and the length
/// in its mapping object [OFW-4.93 lv1 a:0x225E78-0x225EB4], and the
/// address again at +80 [OFW-4.93 lv1 a:0x228DD4]. CellGov's hypervisor
/// runs one partition, so the state keeps no partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MmioMapping {
    /// The bus id, from r3.
    pub bus: u64,
    /// The device id on the bus, from r4.
    pub device: u64,
    /// The bus address of the region, from r5.
    pub bus_address: u64,
    /// The length in bytes, from r6.
    pub size: u64,
    /// The logical partition address that the answer gave in r4.
    pub lpar_address: u64,
}

/// Fields 1 to 5: the bus id, the device id, the bus address, the length
/// and the logical partition address.
impl LaneValue for MmioMapping {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.bus);
        lanes.lane(2, 0, self.device);
        lanes.lane(3, 0, self.bus_address);
        lanes.lane(4, 0, self.size);
        lanes.lane(5, 0, self.lpar_address);
    }
}

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("a map_device_mmio_region at a firmware whose lv1 handler is not read");

/// The call for bus 2, 3 or 4.
const OTHER_BUS: UnmodelledError = UnmodelledError::new(
    "a map_device_mmio_region for bus 2, 3 or 4, whose path through lv1's map function is not \
     read in full",
);

/// The call for a device of bus 1 above device 2.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "a map_device_mmio_region for a device of bus 1 other than devices 0, 1 and 2, whose lv1 \
     lookup is not read",
);

/// The call for an open device whose regions no source gives.
const UNREAD_REGIONS: UnmodelledError = UnmodelledError::new(
    "a map_device_mmio_region for an open device of bus 1 whose MMIO regions no capture reads",
);

/// A call after the partition holds a mapping of the region.
const NEXT_MAPPING: UnmodelledError = UnmodelledError::new(
    "a map_device_mmio_region of a region that the partition already maps, whose logical \
     partition address depends on lv1's allocator state",
);

/// A call whose shape no console capture gives.
const OTHER_SHAPE: UnmodelledError = UnmodelledError::new(
    "a map_device_mmio_region whose region, size and page shift no console capture gives, whose \
     logical partition address depends on lv1's allocator state",
);

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 172: maps the MMIO region at bus address r5 of device r4 of bus r3.
///
/// r6 holds the size and r7 the page shift. A call that fails one of
/// lv1's checks gets lv1's own r3 and r4 to r10 cleared. See
/// [`cellgov_ps3_abi::lv1::device_bus`] for the checks and their order.
/// A call that passes them gets 0 in r3, the logical partition address
/// in r4 and r5 to r10 cleared, and the answer carries the mapping.
///
/// lv1 takes the logical partition address from an allocator whose state
/// CellGov does not model. The answer gives the captured address to the
/// partition's first mapping of the region, for the captured size and
/// page shift.
///
/// The model makes these assumptions:
/// - The console's lv1 made the captured mapping for the kernel's first
///   call, and the kernel did not unmap and map the region again.
/// - The address function's checks of lv1's own state pass for that
///   first call, as the captured mapping shows.
/// - lv1's allocation of its 96-byte mapping object succeeds.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 is bus 2, 3 or 4;
/// - r4 names a device of bus 1 above device 2;
/// - the device is open, and no capture reads its regions;
/// - the partition already maps the region;
/// - no console capture gives the call's shape.
pub(crate) fn map_mmio(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &DeviceBusState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| mmio_map_handler(fw).is_some()) else {
        return UNREAD_FIRMWARE.refusal(request);
    };
    let (bus, device, bus_address, size, page_shift) = (
        request.r(3),
        request.r(4),
        request.r(5),
        request.r(6),
        request.r(7),
    );
    if bus > OPEN_DEVICE_BUS_MAX {
        return lv1_answer(MMIO_MAP_BUS_RANGE_R3);
    }
    if bus == 0 {
        return lv1_answer(MMIO_MAP_NO_BUS_R3);
    }
    if bus != OPEN_DEVICE_READ_BUS {
        return OTHER_BUS.refusal(request);
    }
    if !open_device_path_read(bus, device) {
        return OTHER_DEVICE.refusal(request);
    }
    if !state.is_open(OpenDevice { bus, device }) {
        return lv1_answer(MMIO_MAP_NOT_OPEN_R3);
    }
    if device == 0 {
        return lv1_answer(MMIO_MAP_DEVICE_0_R3);
    }
    if !mmio_regions_read(firmware, bus, device) {
        return UNREAD_REGIONS.refusal(request);
    }
    let Some(region) = mmio_region(firmware, bus, device, bus_address) else {
        return lv1_answer(MMIO_MAP_NO_REGION_R3);
    };
    if size == 0 || size > region.size {
        return lv1_answer(MMIO_MAP_SIZE_R3);
    }
    if !MMIO_MAP_PAGE_SHIFTS.contains(&page_shift) {
        return lv1_answer(MMIO_MAP_PAGE_SHIFT_R3);
    }
    if state.mmio_mapping(bus, device, bus_address).is_some() {
        return NEXT_MAPPING.refusal(request);
    }
    let Some(row) = game_os_mmio_mapping(firmware, bus, device, bus_address, size, page_shift)
    else {
        return OTHER_SHAPE.refusal(request);
    };
    let mapping = MmioMapping {
        bus,
        device,
        bus_address,
        size,
        lpar_address: row.lpar_address,
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [row.lpar_address]);
    answer.push_change(DeviceBusChange::MmioRegionMapped(mapping));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/mmio_map_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/console_mmio_map_tests.rs"]
mod console_tests;
