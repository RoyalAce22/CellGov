//! Hypercall 175, `free_device_dma_region`: lv1 frees a DMA region that
//! `allocate_device_dma_region` gave the calling partition.
//!
//! The handler answers the call for devices 0, 1 and 2 of bus 1; see
//! [`cellgov_ps3_abi::lv1::device_bus`].

use cellgov_ps3_abi::lv1::call::FREE_DEVICE_DMA_REGION;
use cellgov_ps3_abi::lv1::device_bus::{
    dma_free_handler, lv1_dma_region, DMA_FREE_ADDRESS_R3, DMA_FREE_BUSY_R3, DMA_FREE_BUS_RANGE_R3,
    DMA_FREE_NOT_OPEN_R3, DMA_FREE_NO_BUS_R3, DMA_FREE_NO_REGION_R3, DMA_MAP_DEVICES,
    OPEN_DEVICE_BUS_MAX, OPEN_DEVICE_READ_BUS,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{DeviceBusChange, DeviceBusState, OpenDevice};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: FREE_DEVICE_DMA_REGION,
    name: "free_device_dma_region",
    cost: None,
    handler: |request, input| free_region(request, input.firmware, input.groups.device_bus()),
}];

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("a free_device_dma_region at a firmware whose lv1 handler is not read");

/// The call for bus 2, 3 or 4.
const OTHER_BUS: UnmodelledError = UnmodelledError::new(
    "a free_device_dma_region for bus 2, 3 or 4, whose lv1 device lookup is not read",
);

/// The call for a device of bus 1 other than devices 0, 1 and 2.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "a free_device_dma_region for a device of bus 1 other than devices 0, 1 and 2, whose lv1 \
     device no console capture gives",
);

/// The call at the address of lv1's own region of the device.
const LV1_REGION: UnmodelledError = UnmodelledError::new(
    "a free_device_dma_region in lv1's own DMA region of the device, whose mappings no console \
     capture gives",
);

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 175: frees the DMA region at bus address r5 of device r4 of
/// bus r3.
///
/// A call that fails one of lv1's checks gets lv1's own r3 and r4 to r10
/// cleared. See [`cellgov_ps3_abi::lv1::device_bus`] for the checks and
/// their order. A call that names the address of a region of the
/// partition that holds no mapping gets 0 in r3 and r4 to r10 cleared.
/// The answer carries the free of the region.
///
/// lv1 finds the region in the device's list and checks no partition. The
/// regions of the partition and lv1's own regions hold distinct bus
/// addresses. So a region of the partition that holds r5 is the region
/// that lv1 finds.
///
/// After the free, the next allocate for the device gets the refusal for
/// a freed pool. No source gives the address that lv1's pool gives then.
///
/// The model makes these assumptions:
/// - The device's region list holds lv1's own captured regions of the
///   device and the regions that the partition allocated, and no other
///   region.
/// - For device 1 or 2, each region of the partition holds one slot with
///   the low word of its address.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 is bus 2, 3 or 4;
/// - r4 names a device of bus 1 other than devices 0, 1 and 2;
/// - no region of the partition holds r5, and r5 is the address of lv1's
///   own region of the device. Another address in that region gets
///   lv1's r3 for an address other than the region's address.
pub(crate) fn free_region(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &DeviceBusState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| dma_free_handler(fw).is_some()) else {
        return UNREAD_FIRMWARE.refusal(request);
    };
    let (bus, device, bus_address) = (request.r(3), request.r(4), request.r(5));
    if bus > OPEN_DEVICE_BUS_MAX {
        return lv1_answer(DMA_FREE_BUS_RANGE_R3);
    }
    if bus == 0 {
        return lv1_answer(DMA_FREE_NO_BUS_R3);
    }
    if bus != OPEN_DEVICE_READ_BUS {
        return OTHER_BUS.refusal(request);
    }
    if !DMA_MAP_DEVICES.contains(&device) {
        return OTHER_DEVICE.refusal(request);
    }
    let open_device = OpenDevice { bus, device };
    if !state.is_open(open_device) {
        return lv1_answer(DMA_FREE_NOT_OPEN_R3);
    }
    let held = state
        .regions_of(open_device)
        .find(|region| bus_address >= region.address && bus_address - region.address < region.size);
    let Some(region) = held else {
        let Some(own) = lv1_dma_region(firmware, bus, device, bus_address) else {
            return lv1_answer(DMA_FREE_NO_REGION_R3);
        };
        // lv1's find returns lv1's own region, and the address check at
        // +48 runs before the free method reads the region's mappings
        // [OFW-4.93 lv1 a:0x22799C-0x2279C0] [OFW-4.93 lv1 a:0x2262AC-0x2262B4].
        if bus_address != own.address {
            return lv1_answer(DMA_FREE_ADDRESS_R3);
        }
        return LV1_REGION.refusal(request);
    };
    if bus_address != region.address {
        return lv1_answer(DMA_FREE_ADDRESS_R3);
    }
    let mapped = state.mappings_of(open_device).any(|mapping| {
        mapping.bus_address >= region.address && mapping.bus_address - region.address < region.size
    });
    if mapped {
        return lv1_answer(DMA_FREE_BUSY_R3);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(DeviceBusChange::DmaRegionFreed(region));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/dma_free_tests.rs"]
mod tests;
