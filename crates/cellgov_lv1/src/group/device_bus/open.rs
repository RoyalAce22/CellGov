//! Hypercall 170, `open_device`: lv1 opens a device of a bus for the
//! calling partition.
//!
//! The handler answers the call for these opens; see
//! [`cellgov_ps3_abi::lv1::device_bus`]:
//! - devices 0, 1 and 2 of bus 1;
//! - the storage bus object, device -1 of bus 4;
//! - each device of bus 4 above 6, other than -1, which lv1's lookup
//!   finds absent whatever the storage bus object holds
//!   [OFW-4.93 lv1 a:0x2365E4-0x2365F8];
//! - the bus ids that the handler's checks refuse.
//!
//! lv1 finds each other device of buses 1 to 3, and devices 0 to 6 of
//! bus 4, through state that CellGov does not model. A call for one of
//! these devices is a refusal.

use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::OPEN_DEVICE;
use cellgov_ps3_abi::lv1::device_bus::{
    open_device_bus_r3, open_device_handler, open_device_path_read, opens_storage_bus,
    storage_bus_device_absent, OPEN_DEVICE_BUSY_R3, OPEN_DEVICE_NO_DEVICE_R3,
    OPEN_STORAGE_BUS_BUSY_R3,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::close::NETWORK_DEVICE;
use super::{DeviceBusChange, DeviceBusState};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::group::network::NetworkChange;
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

/// The storage bus open that the partition holds.
///
/// lv1 keeps the caller's partition and the call's r5 in a slot of the
/// storage bus object [OFW-4.93 lv1 a:0x254710-0x25472C]. CellGov's
/// hypervisor runs one partition, so the state keeps no partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageBusOpen {
    /// The r5 of the open, which the slot keeps.
    pub r5: u64,
}

/// Field 1: the r5 of the open.
impl LaneValue for StorageBusOpen {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.r5);
    }
}

/// The open of a device that lv1 finds through state that CellGov does
/// not model.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "an open of a device of buses 1 to 3 other than devices 0, 1 and 2 of bus 1, or of devices 0 \
     to 6 of bus 4, which lv1 finds through state that CellGov does not model",
);

/// The open at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("an open_device at a firmware whose lv1 handler is not read");

/// Hypercall 170: opens device r4 of bus r3 for the calling partition.
///
/// The answer clears r4 to r10. Its r3 is one of these values:
/// - the r3 of a failed bus check, from [`open_device_bus_r3`];
/// - for device 0, 1 or 2 of bus 1 that is open, [`OPEN_DEVICE_BUSY_R3`];
/// - for device 0, 1 or 2 of bus 1 that is not open, 0, and the answer
///   opens the device. For device 0 the answer also carries the setup of
///   the network device, as lv1's open hook stores 2 in its state word
///   [OFW-4.93 lv1 a:0x23E1C8-0x23E1D4];
/// - for the storage bus, the r3 of [`open_storage_bus`];
/// - for a device of bus 4 above 6, other than -1,
///   [`OPEN_DEVICE_NO_DEVICE_R3`], and the answer changes no state.
///
/// lv1 reads no r5 on the paths of bus 1 and on the path of an absent
/// device.
///
/// The model assumes that the open hook of device 0 succeeds. When the
/// hook fails, lv1 closes the device again and returns the hook's status
/// [OFW-4.93 lv1 a:0x2377B4-0x2377D8].
///
/// The call is a refusal in these cases:
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
    if opens_storage_bus(request.r(3), request.r(4)) {
        return open_storage_bus(request.r(5), state);
    }
    if storage_bus_device_absent(request.r(3), request.r(4)) {
        return HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            OPEN_DEVICE_NO_DEVICE_R3,
            [],
        ));
    }
    let device = OpenDevice {
        bus: request.r(3),
        device: request.r(4),
    };
    if !open_device_path_read(device.bus, device.device) {
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
    if device == NETWORK_DEVICE {
        answer.push_change(NetworkChange::SetUp);
    }
    HypercallDisposition::Answered(answer)
}

/// Opens the storage bus with the call's `r5`.
///
/// The r3 is one of these values:
/// - when the partition holds the storage bus open,
///   [`OPEN_STORAGE_BUS_BUSY_R3`];
/// - otherwise 0, and the answer carries the open with `r5`.
///
/// The model assumes that lv1's checks pass and that its allocation
/// succeeds, as the capture of the 4.93 boot shows.
fn open_storage_bus(r5: u64, state: &DeviceBusState) -> HypercallDisposition {
    if state.storage_bus_open().is_some() {
        return HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            OPEN_STORAGE_BUS_BUSY_R3,
            [],
        ));
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(DeviceBusChange::StorageBusOpened(StorageBusOpen { r5 }));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/open_device_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/console_open_device_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/open_device_1_tests.rs"]
mod device_1_tests;

#[cfg(test)]
#[path = "tests/console_open_device_1_tests.rs"]
mod console_device_1_tests;

#[cfg(test)]
#[path = "tests/open_device_2_tests.rs"]
mod device_2_tests;

#[cfg(test)]
#[path = "tests/console_open_device_2_tests.rs"]
mod console_device_2_tests;

#[cfg(test)]
#[path = "tests/open_storage_bus_tests.rs"]
mod storage_bus_tests;

#[cfg(test)]
#[path = "tests/console_open_storage_bus_tests.rs"]
mod console_storage_bus_tests;

#[cfg(test)]
#[path = "tests/open_absent_device_tests.rs"]
mod absent_device_tests;
