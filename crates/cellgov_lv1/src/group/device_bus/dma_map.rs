//! Hypercall 176, `map_device_dma_region`: lv1 maps memory of the
//! partition at bus addresses of a DMA region of a device.
//!
//! The handler answers the call for devices 0, 1 and 2 of bus 1; see
//! [`cellgov_ps3_abi::lv1::device_bus`]. The state holds each mapping
//! that lv1 made for the partition.

use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::MAP_DEVICE_DMA_REGION;
use cellgov_ps3_abi::lv1::device_bus::{
    dma_map_alignment_r3, dma_map_handler, dma_map_range_free, is_game_os_dma_mapping,
    lv1_dma_region, DMA_MAP_BUSY_R3, DMA_MAP_BUS_RANGE_R3, DMA_MAP_DEVICES, DMA_MAP_FLAGS_MASK,
    DMA_MAP_FLAGS_R3, DMA_MAP_LENGTH_R3, DMA_MAP_NOT_OPEN_R3, DMA_MAP_NO_BUS_R3,
    DMA_MAP_NO_REGION_R3, OPEN_DEVICE_BUS_MAX,
};
use cellgov_ps3_abi::lv1::network::{STATUS_INDICATOR_BUS, STATUS_INDICATOR_DEVICE};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{DeviceBusChange, DeviceBusState, OpenDevice};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::group::memory::region::{real_mode_size, MemoryState};
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: MAP_DEVICE_DMA_REGION,
    name: "map_device_dma_region",
    cost: None,
    handler: |request, input| {
        map_region(
            request,
            input.firmware,
            input.groups.device_bus(),
            input.groups.memory().regions(),
        )
    },
}];

/// Device 0 of bus 1, the network device.
const NETWORK_DEVICE: OpenDevice = OpenDevice {
    bus: STATUS_INDICATOR_BUS,
    device: STATUS_INDICATOR_DEVICE,
};

/// One mapping that lv1 made in a DMA region of the partition.
///
/// lv1 keeps the partition, the offset of the bus address in the region,
/// the logical partition address and the length in its mapping object
/// [OFW-4.93 lv1 a:0x225E78-0x225EB4]. CellGov's hypervisor runs one
/// partition, so the state keeps no partition. The state keeps no IOPTE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DmaMapping {
    /// The bus id, from r3.
    pub bus: u64,
    /// The device id on the bus, from r4.
    pub device: u64,
    /// The bus address of the mapping's first byte, from r6.
    pub bus_address: u64,
    /// The logical partition address of the memory, from r5.
    pub lpar_address: u64,
    /// The length in bytes, from r7.
    pub length: u64,
}

/// Fields 1 to 5: the bus id, the device id, the bus address, the
/// logical partition address and the length.
impl LaneValue for DmaMapping {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.bus);
        lanes.lane(2, 0, self.device);
        lanes.lane(3, 0, self.bus_address);
        lanes.lane(4, 0, self.lpar_address);
        lanes.lane(5, 0, self.length);
    }
}

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("a map_device_dma_region at a firmware whose lv1 handler is not read");

/// The call for bus 2, 3 or 4.
const OTHER_BUS: UnmodelledError =
    UnmodelledError::new("a map_device_dma_region for bus 2, 3 or 4, whose lv1 path is not read");

/// The call for a device of bus 1 other than devices 0, 1 and 2.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "a map_device_dma_region for a device of bus 1 other than devices 0, 1 and 2, whose lv1 \
     device no console capture gives",
);

/// A call in lv1's own region of the device past the length check.
const LV1_REGION: UnmodelledError = UnmodelledError::new(
    "a map_device_dma_region in lv1's own DMA region of the device, whose mappings no console \
     capture gives",
);

/// A call whose memory is outside the partition memory that CellGov
/// holds.
const OUTSIDE_MEMORY: UnmodelledError = UnmodelledError::new(
    "a map_device_dma_region whose memory is outside the partition memory that CellGov holds, \
     whose lv1 IOPTE path is not read",
);

/// A call that passes lv1's checks and that no console capture shows lv1
/// completed.
const UNCAPTURED_MAP: UnmodelledError = UnmodelledError::new(
    "a map_device_dma_region for device 0, 1 or 2 of bus 1 that no console capture shows, \
     whose lv1 IOPTE writes read IOMMU state that no lv1 read or console capture gives",
);

/// The region that holds the bus address of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HeldRegion {
    /// The region's bus address.
    address: u64,
    /// The region's size in bytes.
    size: u64,
    /// The region's page shift.
    page_shift: u64,
    /// Whether the region is lv1's own region rather than one that the
    /// partition allocated.
    lv1: bool,
}

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// The region of `device` that holds `bus_address`, or `None` when no
/// region holds it.
///
/// The device's list holds lv1's own regions of the device and the
/// regions that the partition allocated, as the captures show.
fn held_region(
    firmware: &str,
    state: &DeviceBusState,
    device: OpenDevice,
    bus_address: u64,
) -> Option<HeldRegion> {
    let partition = state
        .regions_of(device)
        .find(|region| region.address <= bus_address && bus_address - region.address < region.size);
    if let Some(region) = partition {
        return Some(HeldRegion {
            address: region.address,
            size: region.size,
            page_shift: region.page_shift,
            lv1: false,
        });
    }
    lv1_dma_region(firmware, device.bus, device.device, bus_address).map(|region| HeldRegion {
        address: region.address,
        size: region.size,
        page_shift: region.page_shift,
        lv1: true,
    })
}

/// Whether the `length` bytes at `lpar_address` are in partition memory.
///
/// The partition memory that CellGov holds is the real mode region at
/// base 0 and each region that `allocate_memory` created.
fn in_partition_memory(
    firmware: &str,
    memory: &MemoryState,
    lpar_address: u64,
    length: u64,
) -> bool {
    let Some(end) = lpar_address.checked_add(length) else {
        return false;
    };
    if real_mode_size(firmware).is_some_and(|size| end <= size) {
        return true;
    }
    memory.regions().iter().any(|region| {
        region.lpar_address <= lpar_address
            && region
                .lpar_address
                .checked_add(region.size)
                .is_some_and(|region_end| end <= region_end)
    })
}

/// Hypercall 176: maps r7 bytes at logical partition address r5 to bus address r6.
///
/// The call names device r4 of bus r3, and r8 holds the IOPTE flags.
/// A call that fails one of lv1's checks gets lv1's own r3 and r4 to r10
/// cleared. See [`cellgov_ps3_abi::lv1::device_bus`] for the checks and
/// their order. A call that passes them gets 0 in r3 and r4 to r10
/// cleared, and the answer carries the mapping.
///
/// lv1 runs the same checks for devices 0, 1 and 2 with each device's
/// own regions and mappings. So two devices can map the same bus address.
///
/// The model makes these assumptions:
/// - The device's region list holds lv1's own captured regions of the
///   device and the regions that the partition allocated, and no other
///   region.
/// - The device's entry at +120 gives each region an I/O address, as the
///   captures of the word at +160 and the regions' bus addresses show.
/// - lv1's IOPTE writes succeed for a call that [`is_game_os_dma_mapping`]
///   holds. The console's record of the kernel's calls shows each of them.
/// - lv1's allocation of its 96-byte mapping object succeeds.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 is bus 2, 3 or 4;
/// - r4 names a device of bus 1 other than devices 0, 1 and 2;
/// - r6 is in lv1's own region, and the call passes the length check;
/// - the partition memory that CellGov holds does not hold the range;
/// - [`is_game_os_dma_mapping`] does not hold the call.
pub(crate) fn map_region(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &DeviceBusState,
    memory: &MemoryState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| dma_map_handler(fw).is_some()) else {
        return UNREAD_FIRMWARE.refusal(request);
    };
    let (bus, device, lpar_address, bus_address, length, flags) = (
        request.r(3),
        request.r(4),
        request.r(5),
        request.r(6),
        request.r(7),
        request.r(8),
    );
    if bus > OPEN_DEVICE_BUS_MAX {
        return lv1_answer(DMA_MAP_BUS_RANGE_R3);
    }
    if flags & !DMA_MAP_FLAGS_MASK != 0 {
        return lv1_answer(DMA_MAP_FLAGS_R3);
    }
    if bus == 0 {
        return lv1_answer(DMA_MAP_NO_BUS_R3);
    }
    if bus != NETWORK_DEVICE.bus {
        return OTHER_BUS.refusal(request);
    }
    if !DMA_MAP_DEVICES.contains(&device) {
        return OTHER_DEVICE.refusal(request);
    }
    let open_device = OpenDevice { bus, device };
    if !state.is_open(open_device) {
        return lv1_answer(DMA_MAP_NOT_OPEN_R3);
    }
    let Some(region) = held_region(firmware, state, open_device, bus_address) else {
        return lv1_answer(DMA_MAP_NO_REGION_R3);
    };
    if length == 0 || length > region.size {
        return lv1_answer(DMA_MAP_LENGTH_R3);
    }
    if region.lv1 {
        return LV1_REGION.refusal(request);
    }
    let mappings = state
        .mappings_of(open_device)
        .filter(|mapping| {
            mapping.bus_address >= region.address
                && mapping.bus_address - region.address < region.size
        })
        .map(|mapping| (mapping.bus_address - region.address, mapping.length));
    if !dma_map_range_free(region.address, region.size, mappings, bus_address, length) {
        return lv1_answer(DMA_MAP_BUSY_R3);
    }
    if let Some(r3) = dma_map_alignment_r3(bus_address, lpar_address, length, region.page_shift) {
        return lv1_answer(r3);
    }
    if !in_partition_memory(firmware, memory, lpar_address, length) {
        return OUTSIDE_MEMORY.refusal(request);
    }
    if !is_game_os_dma_mapping(
        firmware,
        bus,
        device,
        bus_address,
        lpar_address,
        length,
        flags,
    ) {
        return UNCAPTURED_MAP.refusal(request);
    }
    let mapping = DmaMapping {
        bus,
        device,
        bus_address,
        lpar_address,
        length,
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(DeviceBusChange::DmaRegionMapped(mapping));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/dma_map_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/dma_map_slot_devices_tests.rs"]
mod slot_devices_tests;

#[cfg(test)]
#[path = "tests/dma_map_uncaptured_tests.rs"]
mod uncaptured_tests;

#[cfg(test)]
#[path = "tests/dma_map_envelope_tests.rs"]
mod envelope_tests;

#[cfg(test)]
#[path = "tests/dma_map_late_boot_tests.rs"]
mod late_boot_tests;
