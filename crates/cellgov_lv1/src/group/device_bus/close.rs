//! Hypercall 171, `close_device`: lv1 closes a device of a bus that the
//! calling partition holds open.
//!
//! The handler answers the call for these closes; see
//! [`cellgov_ps3_abi::lv1::device_bus`]:
//! - devices 0, 1 and 2 of bus 1;
//! - the storage bus object, device -1 of bus 4, when the partition does
//!   not hold it open;
//! - each device of bus 4 above 6, other than -1, which lv1's access
//!   check finds absent whatever the storage bus object holds;
//! - the bus ids that the handler's checks refuse.
//!
//! lv1 finds each other device of buses 1 to 3, and devices 0 to 6 of
//! bus 4, through state that CellGov does not model. A call for one of
//! these devices is a refusal. lv1's close of the storage bus reads the
//! state of the storage devices. So a close of a storage bus that the
//! partition holds open is a refusal.

use cellgov_ps3_abi::lv1::call::CLOSE_DEVICE;
use cellgov_ps3_abi::lv1::device_bus::{
    close_device_check_r3, close_device_handler, open_device_path_read, opens_storage_bus,
    CLOSE_DEVICE_NOT_OPEN_R3,
};
use cellgov_ps3_abi::lv1::network::{STATUS_INDICATOR_BUS, STATUS_INDICATOR_DEVICE};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{DeviceBusChange, DeviceBusState, OpenDevice};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::group::network::NetworkChange;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: CLOSE_DEVICE,
    name: "close_device",
    cost: None,
    handler: |request, input| close_device(request, input.firmware, input.groups.device_bus()),
}];

/// The network device: device 0 of bus 1.
pub(super) const NETWORK_DEVICE: OpenDevice = OpenDevice {
    bus: STATUS_INDICATOR_BUS,
    device: STATUS_INDICATOR_DEVICE,
};

/// The close of a device that lv1 finds through state that CellGov does
/// not model.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "a close of a device of buses 1 to 3 other than devices 0, 1 and 2 of bus 1, or of devices 0 \
     to 6 of bus 4, which lv1 finds through state that CellGov does not model",
);

/// The close of the storage bus that the partition holds open.
const STORAGE_BUS: UnmodelledError = UnmodelledError::new(
    "a close of the storage bus that the partition holds open, whose lv1 path reads the state of \
     the storage devices and their requests",
);

/// The close at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("a close_device at a firmware whose lv1 handler is not read");

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 171: closes device r4 of bus r3 for the calling partition.
///
/// The answer clears r4 to r10. Its r3 is one of these values:
/// - the r3 of a failed check that reads no state, from
///   [`close_device_check_r3`];
/// - for a device that the partition does not hold open,
///   [`CLOSE_DEVICE_NOT_OPEN_R3`], and the answer changes no state;
/// - for device 0, 1 or 2 of bus 1 that is open, 0, and the answer
///   closes the device.
///
/// The close of a device removes the device's DMA mappings, frees its
/// DMA regions and removes its MMIO mappings. The close of device 0 also
/// carries the teardown of the network device, which
/// [`NetworkChange::TornDown`] describes.
///
/// The model makes these assumptions about device 0:
/// - The open hook gave the global object its object at +16, as the
///   capture of the open device shows.
/// - That object's flag byte at +0 is set, as the capture of the open
///   network device reads, so the teardown disconnects the port.
/// - The teardown's poll of a hardware register completes.
/// - The teardown's wait on the indicator's memory region ends.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - the partition holds the storage bus open;
/// - the call names another device of a bus that passes the checks.
pub(crate) fn close_device(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &DeviceBusState,
) -> HypercallDisposition {
    if firmware.and_then(close_device_handler).is_none() {
        return UNREAD_FIRMWARE.refusal(request);
    }
    let (bus, device) = (request.r(3), request.r(4));
    if let Some(r3) = close_device_check_r3(bus, device) {
        return lv1_answer(r3);
    }
    if opens_storage_bus(bus, device) {
        if state.storage_bus_open().is_none() {
            return lv1_answer(CLOSE_DEVICE_NOT_OPEN_R3);
        }
        return STORAGE_BUS.refusal(request);
    }
    let device = OpenDevice { bus, device };
    if !open_device_path_read(device.bus, device.device) {
        return OTHER_DEVICE.refusal(request);
    }
    if !state.is_open(device) {
        return lv1_answer(CLOSE_DEVICE_NOT_OPEN_R3);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(DeviceBusChange::Closed(device));
    if device == NETWORK_DEVICE {
        answer.push_change(NetworkChange::TornDown);
    }
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/close_device_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/close_device_state_tests.rs"]
mod state_tests;

#[cfg(test)]
#[path = "tests/console_close_device_tests.rs"]
mod console_tests;
