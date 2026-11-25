//! Hypercall 197, `connect_interrupt_event_receive_port`: lv1 connects an
//! event receive port to the network device's interrupt.
//!
//! The handler answers the call for the network device; see
//! [`cellgov_ps3_abi::lv1::network`]. lv1 keeps one port for that
//! interrupt, and the state holds its id.

use cellgov_ps3_abi::lv1::call::CONNECT_INTERRUPT_EVENT_RECEIVE_PORT;
use cellgov_ps3_abi::lv1::network::{
    interrupt_port_handler, INTERRUPT_PORT_BUSY_R3, INTERRUPT_PORT_BUS_RANGE_R3,
    INTERRUPT_PORT_NOT_OPEN_R3, INTERRUPT_PORT_NO_BUS_R3, INTERRUPT_PORT_NO_PORT_R3,
    INTERRUPT_PORT_R6_R3, STATUS_INDICATOR_BUS, STATUS_INDICATOR_BUS_MAX, STATUS_INDICATOR_DEVICE,
};
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::indicator::NETWORK_DEVICE;
use super::{NetworkChange, NetworkState};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::device_bus::DeviceBusState;
use crate::group::hypercall::Hypercall;
use crate::group::interrupt::event_port::EventReceivePortState;
use crate::group::memory::address_space::AddressSpaceState;
use crate::group::spe::LogicalSpeState;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: CONNECT_INTERRUPT_EVENT_RECEIVE_PORT,
    name: "connect_interrupt_event_receive_port",
    cost: None,
    handler: |request, input| {
        let state = PortState {
            devices: input.groups.device_bus(),
            network: input.groups.network(),
            ports: input.groups.interrupt().ports(),
            spaces: input.groups.memory().spaces(),
            spes: input.groups.spe().spes(),
        };
        connect_port(request, input.firmware, &state)
    },
}];

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError = UnmodelledError::new(
    "a connect_interrupt_event_receive_port at a firmware whose lv1 handler is not read",
);

/// The call for bus 2, 3 or 4.
const OTHER_BUS: UnmodelledError = UnmodelledError::new(
    "a connect_interrupt_event_receive_port for bus 2, 3 or 4, whose lv1 path is not read",
);

/// The call for a device of bus 1 other than the network device.
const OTHER_DEVICE: UnmodelledError = UnmodelledError::new(
    "a connect_interrupt_event_receive_port for a device of bus 1 other than the network device, \
     whose lv1 path is not read",
);

/// The call for an irq outlet.
const OUTLET: UnmodelledError = UnmodelledError::new(
    "a connect_interrupt_event_receive_port for an irq outlet, whose reference in the object \
     table no lv1 reading gives",
);

/// The call for an id that names no object of CellGov's hypervisor.
const UNKNOWN_ID: UnmodelledError = UnmodelledError::new(
    "a connect_interrupt_event_receive_port whose port id names no object CellGov holds",
);

/// The state that the hypervisor holds for the call.
pub(crate) struct PortState<'a> {
    /// The devices that the partition opened.
    pub devices: &'a DeviceBusState,
    /// The network device's state, which holds the connected port.
    pub network: &'a NetworkState,
    /// The event receive ports and the irq outlets.
    pub ports: &'a EventReceivePortState,
    /// The virtual address spaces, whose ids name objects of another
    /// class.
    pub spaces: &'a AddressSpaceState,
    /// The logical SPEs, whose ids name objects of another class.
    pub spes: &'a LogicalSpeState,
}

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 197: connects the port r5 to the interrupt of device r4 of
/// bus r3.
///
/// The answer clears r4 to r10. A call that fails one of lv1's checks
/// gets lv1's own r3; see [`cellgov_ps3_abi::lv1::network`] for the
/// checks and their order. A call that passes them gets 0, and the
/// answer carries the port id.
///
/// The model makes these assumptions:
/// - An open network device has the state word 2, because the open sets
///   it.
/// - Each port of the hypervisor holds a reference, because
///   `construct_event_receive_port` takes one and no answered call drops
///   it.
/// - lv1's allocation of its 88-byte object succeeds.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 is bus 2, 3 or 4;
/// - r4 names a device of bus 1 other than the network device;
/// - r5 names an irq outlet;
/// - r5 names no object that CellGov holds.
pub(crate) fn connect_port(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &PortState<'_>,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| interrupt_port_handler(fw).is_some()) else {
        return UNREAD_FIRMWARE.refusal(request);
    };
    let (bus, device, port_id) = (request.r(3), request.r(4), request.r(5));
    if bus > STATUS_INDICATOR_BUS_MAX {
        return lv1_answer(INTERRUPT_PORT_BUS_RANGE_R3);
    }
    if bus == 0 {
        return lv1_answer(INTERRUPT_PORT_NO_BUS_R3);
    }
    if bus != STATUS_INDICATOR_BUS {
        return OTHER_BUS.refusal(request);
    }
    if device != STATUS_INDICATOR_DEVICE {
        return OTHER_DEVICE.refusal(request);
    }
    if !state.devices.is_open(NETWORK_DEVICE) {
        return lv1_answer(INTERRUPT_PORT_NOT_OPEN_R3);
    }
    if request.r(6) != 0 {
        return lv1_answer(INTERRUPT_PORT_R6_R3);
    }
    if state.network.interrupt_port().is_some() {
        return lv1_answer(INTERRUPT_PORT_BUSY_R3);
    }
    if state.ports.port(port_id).is_none() {
        return not_a_port(request, firmware, state, port_id);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(NetworkChange::InterruptPortConnected(port_id));
    HypercallDisposition::Answered(answer)
}

/// The disposition for a port id `id` that names no port of the
/// hypervisor.
///
/// An id of an object of another class gets
/// [`INTERRUPT_PORT_NO_PORT_R3`]. An irq outlet and an unknown id are
/// refusals.
fn not_a_port(
    request: &HypercallRequest,
    firmware: &str,
    state: &PortState<'_>,
    id: u64,
) -> HypercallDisposition {
    if state.ports.outlet(id).is_some() {
        return OUTLET.refusal(request);
    }
    if game_os_logical_ppe_id(firmware) == Some(id)
        || state.spaces.space(id).is_some()
        || state.spes.spe(id).is_some()
    {
        return lv1_answer(INTERRUPT_PORT_NO_PORT_R3);
    }
    UNKNOWN_ID.refusal(request)
}

#[cfg(test)]
#[path = "tests/interrupt_port_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/interrupt_port_console_tests.rs"]
mod console_tests;
