//! The storage bus path of hypercall 197,
//! `connect_interrupt_event_receive_port`: lv1 connects an event receive
//! port to the partition's slot of the storage bus object or of a storage
//! device object.
//!
//! The facts are in the storage bus and storage device sections of
//! [`cellgov_ps3_abi::lv1::network`]. lv1 keeps the port in the slot's
//! handle of the object, and the device bus state holds its id.

use cellgov_ps3_abi::lv1::device_bus::STORAGE_BUS_DEVICE;
use cellgov_ps3_abi::lv1::network::{
    storage_device_port_read, STORAGE_PORT_BUSY_R3, STORAGE_PORT_NOT_OPEN_R3,
    STORAGE_PORT_NO_DEVICE_R3, STORAGE_PORT_NO_PORT_R3, STORAGE_PORT_R6_R3,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_ps3_abi::lv1::storage::STORAGE_DEVICE_ID_MAX;

use super::interrupt_port::{lv1_answer, not_a_port, PortState};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::device_bus::{DeviceBusChange, StorageDevicePort};
use crate::request::HypercallRequest;

/// The call for a device of bus 4 from 0 to 6 that the firmware's storage
/// bus table holds no entry for.
///
/// lv1 calls a method of the device's object
/// [OFW-4.93 lv1 a:0x2364F8-0x236530], and no source gives that object.
const STORAGE_DEVICE: UnmodelledError = UnmodelledError::new(
    "a connect_interrupt_event_receive_port for a device of bus 4 from 0 to 6 that the \
     firmware's storage bus table holds no entry for, whose device object CellGov does not model",
);

/// Hypercall 197 for bus 4: connects the port r5 to the partition's slot
/// of the storage bus or of a storage device.
///
/// The answer clears r4 to r10. A call that fails one of lv1's checks
/// gets lv1's own r3, in lv1's order:
/// 1. a device id above 6, other than -1, gets
///    [`STORAGE_PORT_NO_DEVICE_R3`];
/// 2. a partition that did not open the device gets
///    [`STORAGE_PORT_NOT_OPEN_R3`];
/// 3. an r6 other than 0 gets [`STORAGE_PORT_R6_R3`];
/// 4. an id of an object of another class gets
///    [`STORAGE_PORT_NO_PORT_R3`];
/// 5. a call while a port holds the slot gets [`STORAGE_PORT_BUSY_R3`].
///
/// A call that passes them gets 0, and the answer carries the port id.
///
/// The model makes these assumptions:
/// - Each port of the hypervisor has the byte 1 at +148, because its
///   constructor succeeded.
/// - Each port of the hypervisor holds a reference, because
///   `construct_event_receive_port` takes one and no answered call drops
///   it.
/// - The partition's slot is the slot that its open took.
///
/// The call is a refusal in these cases:
/// - r4 names a device from 0 to 6 that the firmware's storage bus table
///   holds no entry for;
/// - r5 names an irq outlet;
/// - r5 names no object that CellGov holds.
pub(super) fn connect_storage_port(
    request: &HypercallRequest,
    firmware: &str,
    state: &PortState<'_>,
) -> HypercallDisposition {
    let (device, port_id) = (request.r(4), request.r(5));
    if device <= STORAGE_DEVICE_ID_MAX {
        if !storage_device_port_read(firmware, device) {
            return STORAGE_DEVICE.refusal(request);
        }
        return connect_storage_device_port(request, firmware, state, device);
    }
    if device != STORAGE_BUS_DEVICE {
        return lv1_answer(STORAGE_PORT_NO_DEVICE_R3);
    }
    if state.devices.storage_bus_open().is_none() {
        return lv1_answer(STORAGE_PORT_NOT_OPEN_R3);
    }
    if request.r(6) != 0 {
        return lv1_answer(STORAGE_PORT_R6_R3);
    }
    if state.ports.port(port_id).is_none() {
        return not_a_port(request, firmware, state, port_id, STORAGE_PORT_NO_PORT_R3);
    }
    if state.devices.storage_bus_port().is_some() {
        return lv1_answer(STORAGE_PORT_BUSY_R3);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(DeviceBusChange::StorageBusPortConnected(port_id));
    HypercallDisposition::Answered(answer)
}

/// Hypercall 197 for storage device `device` of the firmware's storage
/// bus table.
///
/// lv1 runs the checks of the storage bus on the device object, so the
/// answers and their order are those of [`connect_storage_port`]. The
/// model makes these assumptions:
/// - The partition holds the device open while the device bus state holds
///   its open.
/// - The partition's slot holds a port while that state holds a port for
///   the device.
fn connect_storage_device_port(
    request: &HypercallRequest,
    firmware: &str,
    state: &PortState<'_>,
    device: u64,
) -> HypercallDisposition {
    let port = request.r(5);
    if state.devices.storage_device_open(device).is_none() {
        return lv1_answer(STORAGE_PORT_NOT_OPEN_R3);
    }
    if request.r(6) != 0 {
        return lv1_answer(STORAGE_PORT_R6_R3);
    }
    if state.ports.port(port).is_none() {
        return not_a_port(request, firmware, state, port, STORAGE_PORT_NO_PORT_R3);
    }
    if state.devices.storage_device_port(device).is_some() {
        return lv1_answer(STORAGE_PORT_BUSY_R3);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(DeviceBusChange::StorageDevicePortConnected(
        StorageDevicePort { device, port },
    ));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/storage_port_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/storage_port_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/storage_device_port_tests.rs"]
mod storage_device_tests;

#[cfg(test)]
#[path = "tests/storage_device_port_console_tests.rs"]
mod storage_device_console_tests;
