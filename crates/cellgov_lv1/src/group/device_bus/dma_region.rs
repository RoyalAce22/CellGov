//! Hypercall 174, `allocate_device_dma_region`: lv1 gives a device of a
//! bus a region of bus addresses for its DMA.
//!
//! The handler answers the call for devices 0, 1 and 2 of bus 1; see
//! [`cellgov_ps3_abi::lv1::device_bus`]. The state holds each region that
//! the partition allocated.

use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::ALLOCATE_DEVICE_DMA_REGION;
use cellgov_ps3_abi::lv1::device_bus::{
    dma_region_handler, dma_region_shape_r3, game_os_dma_regions, DMA_REGION_BUS_RANGE_R3,
    DMA_REGION_FLAGS_MAX, DMA_REGION_FLAGS_R3, DMA_REGION_FLAGS_RANGE_R3, DMA_REGION_NOT_OPEN_R3,
    DMA_REGION_NO_BUS_R3, DMA_SLOT_COUNT, DMA_SLOT_DEVICES, OPEN_DEVICE_BUS_MAX,
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

/// Device 0 of bus 1, the network device.
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

/// The call for a device of bus 1 other than devices 0, 1 and 2.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region for a device of bus 1 other than devices 0, 1 and 2, whose \
     lv1 path is not read",
);

/// A call after the partition holds each captured region of the device.
const NEXT_REGION: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region after the partition holds each region of the device that a \
     console capture gives, whose address depends on lv1's pool state",
);

/// A call whose size and page shift differ from those of the device's
/// next captured region.
const OTHER_SHAPE: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region whose size and page shift no console capture gives for the \
     device's next region, whose address depends on lv1's pool state",
);

/// A call after the partition holds regions of the device other than
/// its first captured regions.
const OTHER_HELD: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region after the partition holds regions of the device other than \
     the first regions of the console capture in their order, whose address depends on lv1's \
     pool state",
);

/// A call for a device after lv1 freed one of its regions.
///
/// A close of the device and `free_device_dma_region` reach the same
/// free method. lv1's free calls the device's method at +112 with the region's address
/// [OFW-4.93 lv1 a:0x2276C8-0x2276E4]. That method reaches the pool's
/// release [OFW-4.93 lv1 a:0x22D918-0x22D924]
/// [OFW-4.93 lv1 a:0x22CEE0-0x22CEE4]. The release passes the address to
/// the free function of the object that the pool's word at +160 selects
/// [OFW-4.93 lv1 a:0x22CA78-0x22CB08]. When a block of the object holds
/// no other entry, that function returns the block's range through the
/// method at +32 of the object [OFW-4.93 lv1 a:0x22AC18-0x22AC3C]. That
/// method passes each I/O segment of the range to a function that works
/// on lv1's global segment table [OFW-4.93 lv1 a:0x22A660-0x22A684]
/// [OFW-4.93 lv1 a:0x2D890C-0x2D8990].
///
/// The next search that finds no gap takes the first free segment of the
/// device's own range [OFW-4.93 lv1 a:0x22B0B8-0x22B0E8]
/// [OFW-4.93 lv1 a:0x2D9680-0x2D96A8]. The free gives back a segment only
/// when a call on the segment's object returns 0
/// [OFW-4.93 lv1 a:0x2D8AA8-0x2D8AB4]. For a segment that has an owner,
/// a lookup of that owner must also succeed [OFW-4.93 lv1 a:0x2D8A50]
/// [OFW-4.93 lv1 a:0x2D8C18-0x2D8C44]. Each condition fails after the
/// owner's listeners get event 3 [OFW-4.93 lv1 a:0x2D7658-0x2D7680]
/// [OFW-4.93 lv1 a:0x2D76A4-0x2D772C] [OFW-4.93 lv1 a:0x2D714C-0x2D7178]
/// [OFW-4.93 lv1 a:0x2D72D4-0x2D7308]. No read shows which code sends that
/// event. So no source shows the address after the free.
///
/// The refusal holds for the device of the freed region alone. lv1's init gives the
/// word at +160 the value 2 for device 0, 8 for device 1 and 9 for
/// device 2 [OFW-4.93 lv1 a:0x3A9924-0x3A9938]
/// [OFW-4.93 lv1 a:0x3A9A4C-0x3A9A5C] [OFW-4.93 lv1 a:0x3A9AA0-0x3A9AB0].
/// Each value selects an allocator with ranges of its own
/// [OFW-4.93 lv1 a:0x22CD58-0x22CD90] [OFW-4.93 lv1 a:0x22A1F0-0x22A214],
/// and the ranges of two devices do not overlap. A free gives back only
/// the segment of the freed address [OFW-4.93 lv1 a:0x2D8974-0x2D8990],
/// and the segment pick stays in the caller's range. So a free in one
/// device leaves the next address of each other device, as the console
/// capture of the allocators and the segment table shows.
const FREED_POOL: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region for a device after lv1 freed one of its regions, whose \
     address depends on the pool state that lv1's free leaves",
);

/// A call for device 1 or 2 after the partition holds all of its slots.
const SLOTS_TAKEN: UnmodelledError = UnmodelledError::new(
    "an allocate_device_dma_region for device 1 or 2 of bus 1 after the partition holds all six \
     of its slots, whose answer depends on lv1's pool state",
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
/// lv1 checks that the flags are 0 for device 0 alone. For devices 1 and
/// 2, lv1 accepts flags 0 to 3.
///
/// lv1 picks the address from a pool whose state CellGov does not model.
/// The pool's state depends on the regions that lv1 gave before, so the
/// answer gives the device's captured regions in the order of the calls.
/// A call gets the address of the device's next captured region when its
/// size and page shift are that region's. Each device has a pool of its
/// own, so the regions of two devices can have the same address.
///
/// The model makes these assumptions:
/// - lv1's init leaves +120 of the device at a value other than -1, as
///   the captures show.
/// - The partition's regions of the device get the captured addresses in
///   the captured order while lv1 freed no region of the device. For
///   device 0, lv1's own region of the device is in the pool before the
///   kernel's call.
/// - lv1's allocation of its 80-byte region object succeeds.
/// - For device 1 or 2, each region of the partition holds one slot, and
///   lv1 holds none.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 is bus 2, 3 or 4;
/// - r4 names a device of bus 1 other than devices 0, 1 and 2;
/// - lv1 freed a region of the device;
/// - for device 1 or 2, the partition holds all six slots;
/// - the partition holds regions of the device other than its first
///   captured regions;
/// - the partition holds each captured region of the device;
/// - no console capture gives the size and the page shift of the
///   device's next region.
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
    let slot_device = DMA_SLOT_DEVICES.contains(&device);
    if device != NETWORK_DEVICE.device && !slot_device {
        return OTHER_DEVICE.refusal(request);
    }
    let open_device = OpenDevice { bus, device };
    if !state.is_open(open_device) {
        return lv1_answer(DMA_REGION_NOT_OPEN_R3);
    }
    if !slot_device && flags != 0 {
        return lv1_answer(DMA_REGION_FLAGS_R3);
    }
    if let Some(r3) = dma_region_shape_r3(size, page_shift) {
        return lv1_answer(r3);
    }
    if state.pool_freed(open_device) {
        return FREED_POOL.refusal(request);
    }
    let held: Vec<DmaRegion> = state.regions_of(open_device).collect();
    if slot_device && held.len() >= DMA_SLOT_COUNT {
        return SLOTS_TAKEN.refusal(request);
    }
    let captured: Vec<DmaRegion> = game_os_dma_regions(firmware, bus, device)
        .map(|row| DmaRegion {
            bus,
            device,
            address: row.address,
            size: row.size,
            page_shift: row.page_shift,
        })
        .collect();
    let region = match next_captured_region(&captured, &held) {
        Ok(region) => region,
        Err(error) => return error.refusal(request),
    };
    if (region.size, region.page_shift) != (size, page_shift) {
        return OTHER_SHAPE.refusal(request);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [region.address]);
    answer.push_change(DeviceBusChange::DmaRegionAllocated(region));
    HypercallDisposition::Answered(answer)
}

/// The device's next captured region after the partition's regions
/// `held`.
///
/// `captured` holds the device's captured regions in the order of the
/// calls. `held` holds the partition's regions of the device in address
/// order.
///
/// # Errors
///
/// - `OTHER_HELD` if `held` is not the set of the first captured regions.
/// - `OTHER_SHAPE` if `captured` and `held` are empty.
/// - `NEXT_REGION` if no captured region follows a nonempty `held`.
fn next_captured_region(
    captured: &[DmaRegion],
    held: &[DmaRegion],
) -> Result<DmaRegion, UnmodelledError> {
    let Some(earlier) = captured.get(..held.len()) else {
        return Err(OTHER_HELD);
    };
    let mut earlier = earlier.to_vec();
    earlier.sort_unstable();
    if earlier != held {
        return Err(OTHER_HELD);
    }
    match captured.get(held.len()) {
        Some(&next) => Ok(next),
        None if held.is_empty() => Err(OTHER_SHAPE),
        None => Err(NEXT_REGION),
    }
}

#[cfg(test)]
#[path = "tests/dma_region_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/dma_region_slot_tests.rs"]
mod slot_tests;

#[cfg(test)]
#[path = "tests/dma_region_rows_tests.rs"]
mod rows_tests;

#[cfg(test)]
#[path = "tests/dma_region_device_2_tests.rs"]
mod device_2_tests;

#[cfg(test)]
#[path = "tests/dma_region_reopen_tests.rs"]
mod reopen_tests;

#[cfg(test)]
#[path = "tests/console_freed_pool_tests.rs"]
mod console_freed_pool_tests;
