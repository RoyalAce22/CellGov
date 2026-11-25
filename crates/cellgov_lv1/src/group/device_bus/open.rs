//! Hypercall 170, `open_device`: lv1 opens a device of a bus for the
//! calling partition.
//!
//! The handler answers the call for device 0 of bus 1 and for the bus
//! ids that the handler's checks refuse; see
//! [`cellgov_ps3_abi::lv1::device_bus`]. lv1 finds each other device
//! through state that CellGov does not model, so a call for one is a
//! refusal.

use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::OPEN_DEVICE;
use cellgov_ps3_abi::lv1::device_bus::{
    open_device_bus_r3, open_device_handler, OPEN_DEVICE_BUSY_R3, OPEN_DEVICE_READ_BUS,
    OPEN_DEVICE_READ_DEVICE,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{DeviceBusChange, DeviceBusState};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: OPEN_DEVICE,
    name: "open_device",
    cost: None,
    handler: |request, input| open_device(request, input.firmware, input.groups.device_bus()),
}];

/// One device that the partition opened.
///
/// lv1 keeps the open flag and the owner partition in the device object
/// [OFW-4.93 lv1 a:0x2280B0-0x2280BC]. CellGov's hypervisor runs one
/// partition, so the state keeps no owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OpenDevice {
    /// The bus id, from r3.
    pub bus: u64,
    /// The device id on the bus, from r4.
    pub device: u64,
}

/// Fields 1 and 2: the bus id and the device id.
impl LaneValue for OpenDevice {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.bus);
        lanes.lane(2, 0, self.device);
    }
}

/// The open of a device that is not device 0 of bus 1.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "an open of a device other than device 0 of bus 1, which lv1 finds through state that \
     CellGov does not model",
);

/// The open at a firmware whose handler is not read.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("an open_device at a firmware whose lv1 handler is not read");

/// Hypercall 170: opens device r4 of bus r3 for the calling partition.
///
/// The answer clears r4 to r10. Its r3 is one of these values:
/// - the r3 of a failed bus check, from [`open_device_bus_r3`];
/// - for device 0 of bus 1 that is open, [`OPEN_DEVICE_BUSY_R3`];
/// - for device 0 of bus 1 that is not open, 0, and the answer opens
///   the device.
///
/// lv1 reads no r5 on these paths. The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - the call names another device of a bus that passes the checks.
pub(crate) fn open_device(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &DeviceBusState,
) -> HypercallDisposition {
    if firmware.and_then(open_device_handler).is_none() {
        return UNREAD_FIRMWARE.refusal(request);
    }
    if let Some(r3) = open_device_bus_r3(request.r(3)) {
        return HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []));
    }
    let device = OpenDevice {
        bus: request.r(3),
        device: request.r(4),
    };
    if (device.bus, device.device) != (OPEN_DEVICE_READ_BUS, OPEN_DEVICE_READ_DEVICE) {
        return OTHER_DEVICE.refusal(request);
    }
    if state.is_open(device) {
        return HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            OPEN_DEVICE_BUSY_R3,
            [],
        ));
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(DeviceBusChange::Opened(device));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/open_device_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/console_open_device_tests.rs"]
mod console_tests;
