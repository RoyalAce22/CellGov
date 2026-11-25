//! Hypercall 18, `construct_event_receive_port`, and the event receive
//! ports that the hypervisor keeps for the partition.
//!
//! lv1 reads no input register, and keeps no thread in the port it
//! constructs. So the handler does not read
//! [`HypercallRequest::thread`], and the hypervisor keys each port by its
//! id alone.
//!
//! lv1 takes the port id from the partition's object table, and later
//! interrupt calls look the port up by that id; see
//! [`cellgov_ps3_abi::lv1::event_port`]. So the hypervisor keeps each
//! port, and hashes it.

use std::collections::BTreeMap;

use cellgov_mem::lanes::{self, source, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::event_port::{event_receive_port_handler, game_os_first_port_id};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use super::interrupt::{InterruptChange, IrqStateBitmapKey};
use super::request::HypercallRequest;
use super::vector;

/// One event receive port that the hypervisor constructed.
///
/// `connect_irq_plug_ext` sets the connection and the plug; see
/// [`cellgov_ps3_abi::lv1::irq_plug`]. A connected port holds the plug
/// slot of its thread. No other call that CellGov answers fills a plug
/// slot, so the ports give each thread's plug table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventReceivePort {
    /// The port id in the partition's object table, which the answer put
    /// in r4.
    pub id: u64,
    /// The thread whose plug table the port's +128 names, or `None` for
    /// no connection.
    pub connection: Option<IrqStateBitmapKey>,
    /// The plug at +144 of the port.
    ///
    /// A connect step sets it even when it fails because the plug holds
    /// another port. lv1 reads it only for a connected port.
    pub plug: u8,
}

impl EventReceivePort {
    /// The port with the id `id` as lv1's constructor leaves it: no
    /// connection, and plug 0.
    pub const fn new(id: u64) -> Self {
        Self {
            id,
            connection: None,
            plug: 0,
        }
    }
}

/// These fields:
/// - field 1: the port id;
/// - field 2: 1 for a connection;
/// - fields 3 and 4: the logical PPE id and the thread of the connection;
/// - field 5: the plug.
impl LaneValue for EventReceivePort {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.id);
        if let Some(key) = self.connection {
            lanes.lane(2, 0, 1);
            lanes.lane(3, 0, key.ppe_id);
            lanes.lane(4, 0, u64::from(key.thread));
        }
        lanes.lane(5, 0, u64::from(self.plug));
    }
}

/// The partition's event receive ports, by port id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventReceivePortState {
    ports: BTreeMap<u64, EventReceivePort>,
}

impl EventReceivePortState {
    /// Each port, by its id.
    pub fn ports(&self) -> &BTreeMap<u64, EventReceivePort> {
        &self.ports
    }

    /// The port with the id `id`, or `None` when the hypervisor did not
    /// construct it.
    pub fn port(&self, id: u64) -> Option<&EventReceivePort> {
        self.ports.get(&id)
    }

    /// The port that the plug `plug` of the thread `thread` holds, or
    /// `None` when the plug is free.
    pub fn port_on_plug(&self, thread: IrqStateBitmapKey, plug: u8) -> Option<&EventReceivePort> {
        self.ports
            .values()
            .find(|port| port.connection == Some(thread) && port.plug == plug)
    }

    /// Keeps `port`.
    pub(super) fn construct(&mut self, port: EventReceivePort) {
        self.ports.insert(port.id, port);
    }

    /// Keeps `port` in place of the port with its id, which a connect
    /// step changed.
    pub(super) fn connect(&mut self, port: EventReceivePort) {
        self.ports.insert(port.id, port);
    }

    /// The sync-state terms of the ports.
    ///
    /// Each port is one object of
    /// [`source::HYPERVISOR_EVENT_RECEIVE_PORT`], keyed by its index in id
    /// order. A state with no port adds no term.
    pub fn sync_term(&self) -> u128 {
        (0u64..)
            .zip(self.ports.values())
            .fold(0u128, |sum, (index, port)| {
                sum.wrapping_add(lanes::value_term(
                    source::HYPERVISOR_EVENT_RECEIVE_PORT,
                    index,
                    port,
                ))
            })
    }
}

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: request.number,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// Hypercall 18: a new event receive port, with its id in r4 and r5 to
/// r10 cleared.
///
/// The answer carries the port, and the hypervisor keeps it.
///
/// lv1 gives the port the first free id of the partition's object table.
/// The partition's other objects decide that id, so CellGov answers only
/// the id that a console capture gives: the id of the GameOS kernel's
/// first port.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - the hypervisor already constructed a port, so no source gives the
///   port id;
/// - no source gives the first port id for `firmware`.
pub(super) fn construct_port(
    request: &HypercallRequest,
    firmware: Option<&str>,
    ports: &EventReceivePortState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| event_receive_port_handler(fw).is_some()) else {
        return refuse(
            request,
            "an event receive port call at a firmware whose lv1 handler is not read",
        );
    };
    if !ports.ports.is_empty() {
        return refuse(
            request,
            "a second event receive port, whose id the partition's other objects decide",
        );
    }
    let Some(id) = game_os_first_port_id(firmware) else {
        return refuse(
            request,
            "an event receive port at a firmware whose first port id no source gives",
        );
    };
    let mut outputs = vector::cleared_outputs();
    outputs[0] = id;
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), outputs);
    answer.interrupt = Some(Box::new(InterruptChange::PortConstructed(
        EventReceivePort::new(id),
    )));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/event_port_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/event_port_hash_tests.rs"]
mod hash_tests;
