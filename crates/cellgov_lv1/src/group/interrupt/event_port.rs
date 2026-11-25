//! Hypercall 18, `construct_event_receive_port`, and the event receive
//! ports that the hypervisor keeps for the partition.
//!
//! lv1 reads no input register, and keeps no thread in the port it
//! constructs. So the hypervisor keys each port by its id alone.
//!
//! lv1 takes the port id from the partition's object table, and later
//! interrupt calls look the port up by that id; see
//! [`cellgov_ps3_abi::lv1::event_port`]. So the hypervisor keeps each
//! port, and hashes it.
//!
//! lv1 constructs each irq outlet of a logical SPE with the port
//! constructor, and `connect_irq_plug_ext` connects an outlet as it
//! connects a port; see [`cellgov_ps3_abi::lv1::spe`]. So the state also
//! keeps the outlets, apart from the ports, and hashes them.
//!
//! An event sets the pending byte of a port or irq outlet, and a later
//! call reads it; see [`cellgov_ps3_abi::lv1::send_event`].
//! `end_of_interrupt_ext` clears it; see
//! [`cellgov_ps3_abi::lv1::end_of_interrupt`]. So the state keeps the
//! ports and irq outlets with a pending event, and hashes them.
//!
//! Console captures give the port id of each call of the GameOS kernel's
//! boot, by the call's hardware thread and its `sc` address. So
//! the handler reads [`HypercallRequest::thread`] and
//! [`HypercallRequest::pc`], although lv1 reads neither.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::CONSTRUCT_EVENT_RECEIVE_PORT;
use cellgov_ps3_abi::lv1::event_port::{event_receive_port_handler, game_os_port_id};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::bitmap::IrqStateBitmapKey;
use super::{InterruptChange, EVENT_RECEIVE_PORT_SOURCE, IRQ_OUTLET_SOURCE, PENDING_EVENT_SOURCE};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: CONSTRUCT_EVENT_RECEIVE_PORT,
    name: "construct_event_receive_port",
    cost: None,
    handler: |request, input| {
        construct_port(request, input.firmware, input.groups.interrupt().ports())
    },
}];

/// One event receive port or irq outlet that the hypervisor constructed.
///
/// `connect_irq_plug_ext` sets the connection and the plug; see
/// [`cellgov_ps3_abi::lv1::irq_plug`]. A connected port holds the plug
/// slot of its thread. No other call that CellGov answers fills a plug
/// slot, so the ports and the outlets give each thread's plug table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventReceivePort {
    /// The id in the partition's object table.
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

/// A port or irq outlet with a pending event. Field 1 is its id.
struct PendingPort(u64);

impl LaneValue for PendingPort {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.0);
    }
}

/// The partition's event receive ports and its irq outlets, each by its
/// id, and the ports and outlets that hold a pending event.
///
/// `set_spe_transition_notifier` takes a port and refuses an outlet, so
/// the state keeps the two apart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventReceivePortState {
    ports: BTreeMap<u64, EventReceivePort>,
    outlets: BTreeMap<u64, EventReceivePort>,
    /// The id of each port or outlet whose pending byte at +149 is set.
    pending: BTreeSet<u64>,
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

    /// Each irq outlet, by its id.
    pub fn outlets(&self) -> &BTreeMap<u64, EventReceivePort> {
        &self.outlets
    }

    /// The irq outlet with the id `id`, or `None` when the hypervisor did
    /// not construct it.
    pub fn outlet(&self, id: u64) -> Option<&EventReceivePort> {
        self.outlets.get(&id)
    }

    /// The port or irq outlet that the plug `plug` of the thread `thread`
    /// holds, or `None` when the plug is free.
    pub fn port_on_plug(&self, thread: IrqStateBitmapKey, plug: u8) -> Option<&EventReceivePort> {
        self.ports
            .values()
            .chain(self.outlets.values())
            .find(|port| port.connection == Some(thread) && port.plug == plug)
    }

    /// Keeps `port`.
    pub(crate) fn construct(&mut self, port: EventReceivePort) {
        self.ports.insert(port.id, port);
    }

    /// Keeps `port` in place of the port with its id, which a connect
    /// step changed.
    pub(crate) fn connect(&mut self, port: EventReceivePort) {
        self.ports.insert(port.id, port);
    }

    /// Keeps the irq outlet `outlet`.
    pub(crate) fn construct_outlet(&mut self, outlet: EventReceivePort) {
        self.outlets.insert(outlet.id, outlet);
    }

    /// Keeps `outlet` in place of the irq outlet with its id, which a
    /// connect step changed.
    pub(crate) fn connect_outlet(&mut self, outlet: EventReceivePort) {
        self.outlets.insert(outlet.id, outlet);
    }

    /// Whether the port or irq outlet with the id `id` holds a pending
    /// event.
    pub fn is_pending(&self, id: u64) -> bool {
        self.pending.contains(&id)
    }

    /// Sets the pending byte of the port or irq outlet with the id `id`.
    pub(crate) fn set_pending(&mut self, id: u64) {
        self.pending.insert(id);
    }

    /// Clears the pending byte of the port or irq outlet with the id `id`.
    pub(crate) fn clear_pending(&mut self, id: u64) {
        self.pending.remove(&id);
    }

    /// The sync-state terms of the ports, the irq outlets and the pending
    /// events.
    ///
    /// The objects are of these lane sources:
    /// - each port is one object of `EVENT_RECEIVE_PORT_SOURCE`;
    /// - each outlet is one object of `IRQ_OUTLET_SOURCE`;
    /// - each port or outlet with a pending event is one object of
    ///   `PENDING_EVENT_SOURCE`.
    ///
    /// Each object's key is its index in id order. A state with no port,
    /// no outlet and no pending event adds no term.
    pub fn sync_term(&self) -> u128 {
        let term = |source: u8, objects: &BTreeMap<u64, EventReceivePort>| {
            (0u64..)
                .zip(objects.values())
                .fold(0u128, |sum, (index, object)| {
                    sum.wrapping_add(lanes::value_term(source, index, object))
                })
        };
        let pending = (0u64..)
            .zip(&self.pending)
            .fold(0u128, |sum, (index, &id)| {
                sum.wrapping_add(lanes::value_term(
                    PENDING_EVENT_SOURCE,
                    index,
                    &PendingPort(id),
                ))
            });
        term(EVENT_RECEIVE_PORT_SOURCE, &self.ports)
            .wrapping_add(term(IRQ_OUTLET_SOURCE, &self.outlets))
            .wrapping_add(pending)
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

/// Hypercall 18: a new event receive port, with its id in r4 and r5 to r10 cleared.
///
/// The answer carries the port, and the hypervisor keeps it. lv1 gives
/// the port the first free id of the partition's object table, so the
/// partition's other objects decide that id. CellGov answers the id that
/// a console capture gives for the call's hardware thread and its `sc`
/// address. It gives that id whatever objects the hypervisor holds and
/// whatever order it runs the threads in.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the port id for the thread and the address at
///   `firmware`;
/// - the hypervisor already holds a port or an irq outlet with that id.
pub(crate) fn construct_port(
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
    let Some(id) = request
        .thread
        .and_then(|thread| game_os_port_id(firmware, thread, request.pc))
    else {
        return refuse(
            request,
            "an event receive port call whose port id no source gives for its thread and address",
        );
    };
    // An outlet takes its id from the same object table as a port; see
    // [`cellgov_ps3_abi::lv1::spe`]. So lv1 never gives a port an outlet's id.
    if ports.port(id).is_some() || ports.outlet(id).is_some() {
        return refuse(
            request,
            "an event receive port call whose port id the hypervisor already holds",
        );
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [id]);
    answer.push_change(InterruptChange::PortConstructed(EventReceivePort::new(id)));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/event_port_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/event_port_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/event_port_boot_order_tests.rs"]
mod boot_order_tests;

#[cfg(test)]
#[path = "tests/event_port_spe_tests.rs"]
mod spe_tests;

#[cfg(test)]
#[path = "tests/event_port_outlet_hash_tests.rs"]
mod outlet_hash_tests;

#[cfg(test)]
#[path = "tests/event_port_record_start_tests.rs"]
mod record_start_tests;

#[cfg(test)]
#[path = "tests/event_port_storage_tests.rs"]
mod storage_tests;
