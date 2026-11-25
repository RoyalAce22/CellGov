//! Hypercall 174, `allocate_device_dma_region`: lv1 gives a device of a
//! bus a region of bus addresses for its DMA.
//!
//! The handler answers the call for device 0 of bus 1, the network
//! device; see [`cellgov_ps3_abi::lv1::device_bus`]. The state holds each
//! region that the partition allocated.

use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::ALLOCATE_DEVICE_DMA_REGION;
use cellgov_ps3_abi::lv1::device_bus::{
    dma_region_handler, dma_region_shape_r3, game_os_dma_region, DMA_REGION_BUS_RANGE_R3,
    DMA_REGION_FLAGS_MAX, DMA_REGION_FLAGS_R3, DMA_REGION_FLAGS_RANGE_R3, DMA_REGION_NOT_OPEN_R3,
    DMA_REGION_NO_BUS_R3, OPEN_DEVICE_BUS_MAX,
};
use cellgov_ps3_abi::lv1::network::{STATUS_INDICATOR_BUS, STATUS_INDICATOR_DEVICE};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{DeviceBusChange, DeviceBusState, OpenDevice};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: ALLOCATE_DEVICE_DMA_REGION,
    name: "allocate_device_dma_region",
    cost: None,
    handler: |request, input| allocate_region(request, input.firmware, input.groups.device_bus()),
}];

/// The device whose allocate path CellGov read: device 0 of bus 1, the
/// network device.
const NETWORK_DEVICE: OpenDevice = OpenDevice {
    bus: STATUS_INDICATOR_BUS,
    device: STATUS_INDICATOR_DEVICE,
};

/// One DMA region that lv1 gave the partition.
///
/// lv1 keeps the partition, the address, the size and the page shift in
/// its region object [OFW-4.93 lv1 a:0x225CAC-0x225CDC]. CellGov's
/// hypervisor runs one partition, so the state keeps no partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DmaRegion {
    /// The bus id, from r3.
    pub bus: u64,
    /// The device id on the bus, from r4.
    pub device: u64,
    /// The bus address that the answer gave in r4.
    pub address: u64,
    /// The size in bytes, from r5.
    pub size: u64,
    /// The page shift, from r6.
    pub page_shift: u64,
}

/// Fields 1 to 5: the bus id, the device id, the address, the size and
/// the page shift.
impl LaneValue for DmaRegion {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.bus);
        lanes.lane(2, 0, self.device);
        lanes.lane(3, 0, self.address);
        lanes.lane(4, 0, self.size);
        lanes.lane(5, 0, self.page_shift);
    }
}

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region at a firmware whose lv1 handler is not read",
);

/// The call for bus 2, 3 or 4.
const OTHER_BUS: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region for bus 2, 3 or 4, whose lv1 path is not read",
);

/// The call for a device of bus 1 other than device 0.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region for a device of bus 1 other than device 0, whose lv1 path is \
     not read",
);

/// A call after the partition holds a region of the device.
const NEXT_REGION: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region after the partition holds a region of the device, whose \
     address depends on lv1's pool state",
);

/// A call whose size and page shift no console capture gives.
const OTHER_SHAPE: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region whose size and page shift no console capture gives, whose \
     address depends on lv1's pool state",
);

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 174: allocates a DMA region of r5 bytes with pages of 2^r6 bytes.
///
/// The call names device r4 of bus r3, and r7 holds the flags.
/// A call that fails one of lv1's checks gets lv1's own r3 and r4 to r10
/// cleared. See [`cellgov_ps3_abi::lv1::device_bus`] for the checks and
/// their order. A call that passes them gets 0 in r3, the bus address in
/// r4 and r5 to r10 cleared, and the answer carries the region.
///
/// lv1 picks the address from a pool whose state CellGov does not model.
/// The answer gives the captured address to the device's first region of
/// the captured size and page shift.
///
/// The model makes these assumptions:
/// - lv1's init leaves +120 of the device at a value other than -1, as
///   the capture shows.
/// - The partition's first region of the device gets the captured
///   address. lv1's own region of the device is in the pool before the
///   kernel's call.
/// - lv1's allocation of its 80-byte region object succeeds.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 is bus 2, 3 or 4;
/// - r4 names a device of bus 1 other than device 0;
/// - the partition already holds a region of the device;
/// - no console capture gives the size and the page shift.
pub(crate) fn allocate_region(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &DeviceBusState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| dma_region_handler(fw).is_some()) else {
        return UNREAD_FIRMWARE.refusal(request);
    };
    let (bus, device, size, page_shift, flags) = (
        request.r(3),
        request.r(4),
        request.r(5),
        request.r(6),
        request.r(7),
    );
    if bus > OPEN_DEVICE_BUS_MAX {
        return lv1_answer(DMA_REGION_BUS_RANGE_R3);
    }
    if flags > DMA_REGION_FLAGS_MAX {
        return lv1_answer(DMA_REGION_FLAGS_RANGE_R3);
    }
    if bus == 0 {
        return lv1_answer(DMA_REGION_NO_BUS_R3);
    }
    if bus != NETWORK_DEVICE.bus {
        return OTHER_BUS.refusal(request);
    }
    if device != NETWORK_DEVICE.device {
        return OTHER_DEVICE.refusal(request);
    }
    if !state.is_open(NETWORK_DEVICE) {
        return lv1_answer(DMA_REGION_NOT_OPEN_R3);
    }
    if flags != 0 {
        return lv1_answer(DMA_REGION_FLAGS_R3);
    }
    if let Some(r3) = dma_region_shape_r3(size, page_shift) {
        return lv1_answer(r3);
    }
    if state.regions_of(NETWORK_DEVICE).next().is_some() {
        return NEXT_REGION.refusal(request);
    }
    let Some(address) = game_os_dma_region(firmware, bus, device, size, page_shift) else {
        return OTHER_SHAPE.refusal(request);
    };
    let region = DmaRegion {
        bus,
        device,
        address,
        size,
        page_shift,
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [address]);
    answer.push_change(DeviceBusChange::DmaRegionAllocated(region));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/dma_region_tests.rs"]
mod tests;
