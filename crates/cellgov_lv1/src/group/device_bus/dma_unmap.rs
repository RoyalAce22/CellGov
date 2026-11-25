//! Hypercall 177, `unmap_device_dma_region`: lv1 removes a DMA mapping
//! that `map_device_dma_region` made for the calling partition.
//!
//! The handler answers the call for devices 0, 1 and 2 of bus 1; see
//! [`cellgov_ps3_abi::lv1::device_bus`].

use cellgov_ps3_abi::lv1::call::UNMAP_DEVICE_DMA_REGION;
use cellgov_ps3_abi::lv1::device_bus::{
    dma_unmap_handler, lv1_dma_region, DMA_MAP_DEVICES, DMA_UNMAP_ADDRESS_R3,
    DMA_UNMAP_BUS_RANGE_R3, DMA_UNMAP_LENGTH_R3, DMA_UNMAP_NOT_OPEN_R3, DMA_UNMAP_NO_BUS_R3,
    DMA_UNMAP_NO_MAPPING_R3, OPEN_DEVICE_BUS_MAX, OPEN_DEVICE_READ_BUS,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{DeviceBusChange, DeviceBusState, OpenDevice};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: UNMAP_DEVICE_DMA_REGION,
    name: "unmap_device_dma_region",
    cost: None,
    handler: |request, input| unmap_region(request, input.firmware, input.groups.device_bus()),
}];

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("an unmap_device_dma_region at a firmware whose lv1 handler is not read");

/// The call for bus 2, 3 or 4.
const OTHER_BUS: UnmodelledError = UnmodelledError::new(
    "an unmap_device_dma_region for bus 2, 3 or 4, whose lv1 device lookup is not read",
);

/// The call for a device of bus 1 other than devices 0, 1 and 2.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "an unmap_device_dma_region for a device of bus 1 other than devices 0, 1 and 2, whose lv1 \
     device no console capture gives",
);

/// The call at a bus address in lv1's own region of the device.
const LV1_REGION: UnmodelledError = UnmodelledError::new(
    "an unmap_device_dma_region in lv1's own DMA region of the device, whose mappings no console \
     capture gives",
);

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 177: removes the DMA mapping of r6 bytes at bus address r5
/// of device r4 of bus r3.
///
/// A call that fails one of lv1's checks gets lv1's own r3 and r4 to r10
/// cleared. See [`cellgov_ps3_abi::lv1::device_bus`] for the checks and
/// their order. A call that names the first byte and the length of a
/// mapping of the device gets 0 in r3 and r4 to r10 cleared. The answer
/// carries the removal of the mapping.
///
/// lv1 finds the mapping in each region of the device and checks no
/// partition. The regions of the partition and lv1's own regions hold
/// distinct bus addresses. So a mapping of the partition that holds r5 is
/// the mapping that lv1 finds.
///
/// The model makes these assumptions:
/// - The device's region list holds lv1's own captured regions of the
///   device and the regions that the partition allocated, and no other
///   region.
/// - lv1's IOPTE clears succeed for a mapping whose IOPTE writes
///   succeeded. The clears read the IOMMU state that the writes read,
///   with the same inputs.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 is bus 2, 3 or 4;
/// - r4 names a device of bus 1 other than devices 0, 1 and 2;
/// - no mapping of the partition holds r5, and r5 is in lv1's own region
///   of the device.
pub(crate) fn unmap_region(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &DeviceBusState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| dma_unmap_handler(fw).is_some()) else {
        return UNREAD_FIRMWARE.refusal(request);
    };
    let (bus, device, bus_address, length) =
        (request.r(3), request.r(4), request.r(5), request.r(6));
    if bus > OPEN_DEVICE_BUS_MAX {
        return lv1_answer(DMA_UNMAP_BUS_RANGE_R3);
    }
    if bus == 0 {
        return lv1_answer(DMA_UNMAP_NO_BUS_R3);
    }
    if bus != OPEN_DEVICE_READ_BUS {
        return OTHER_BUS.refusal(request);
    }
    if !DMA_MAP_DEVICES.contains(&device) {
        return OTHER_DEVICE.refusal(request);
    }
    let open_device = OpenDevice { bus, device };
    if !state.is_open(open_device) {
        return lv1_answer(DMA_UNMAP_NOT_OPEN_R3);
    }
    let held = state.mappings_of(open_device).find(|mapping| {
        bus_address >= mapping.bus_address && bus_address - mapping.bus_address < mapping.length
    });
    let Some(mapping) = held else {
        if lv1_dma_region(firmware, bus, device, bus_address).is_some() {
            return LV1_REGION.refusal(request);
        }
        return lv1_answer(DMA_UNMAP_NO_MAPPING_R3);
    };
    if bus_address != mapping.bus_address {
        return lv1_answer(DMA_UNMAP_ADDRESS_R3);
    }
    if length != mapping.length {
        return lv1_answer(DMA_UNMAP_LENGTH_R3);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(DeviceBusChange::DmaRegionUnmapped(mapping));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/dma_unmap_tests.rs"]
mod tests;
