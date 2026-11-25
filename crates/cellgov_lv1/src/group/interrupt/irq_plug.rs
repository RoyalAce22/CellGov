//! Hypercall 12, `connect_irq_plug_ext`: the call connects an event
//! receive port or an irq outlet to a plug of a logical PPE thread.
//!
//! r3 and r4 name the thread. lv1 does not compare r4 with the calling
//! thread, so the handler does not read [`HypercallRequest::thread`].
//! The hypervisor keeps the connection in the port or the outlet; see
//! [`EventReceivePort`].
//!
//! lv1 constructs an outlet with the port constructor, so an outlet
//! passes each check that a port passes; see [`cellgov_ps3_abi::lv1::spe`].

use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::CONNECT_IRQ_PLUG_EXT;
use cellgov_ps3_abi::lv1::interrupt::THREADS_PER_LOGICAL_PPE;
use cellgov_ps3_abi::lv1::irq_plug::{
    irq_plug_handler, IRQ_PLUGS_PER_THREAD, IRQ_PLUG_IN_USE_R3, IRQ_PLUG_PLUG_R3,
    IRQ_PLUG_PORT_CONNECTED_R3, IRQ_PLUG_PORT_R3, IRQ_PLUG_PPE_ID_R3, IRQ_PLUG_R7_R3,
    IRQ_PLUG_THREAD_R3,
};
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::bitmap::{IrqStateBitmapKey, IrqStateBitmapState};
use super::event_port::{EventReceivePort, EventReceivePortState};
use super::raise;
use super::InterruptChange;
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::group::memory::address_space::AddressSpaceState;
use crate::group::spe::LogicalSpeState;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: CONNECT_IRQ_PLUG_EXT,
    name: "connect_irq_plug_ext",
    cost: None,
    handler: |request, input| {
        let interrupt = input.groups.interrupt();
        connect_plug(
            request,
            input.firmware,
            &PlugState {
                spaces: input.groups.memory().spaces(),
                spes: input.groups.spe().spes(),
                bitmaps: interrupt.bitmaps(),
                ports: interrupt.ports(),
                storage: input.storage,
            },
        )
    },
}];

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: request.number,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// The state that the hypervisor holds for a connect step.
pub(crate) struct PlugState<'a> {
    /// The virtual address spaces, whose ids name objects of another
    /// class.
    pub spaces: &'a AddressSpaceState,
    /// The logical SPEs, whose ids name objects of another class.
    pub spes: &'a LogicalSpeState,
    /// The interrupt state bitmaps.
    pub bitmaps: &'a IrqStateBitmapState,
    /// The event receive ports and the irq outlets.
    pub ports: &'a EventReceivePortState,
    /// The caller's real address space, which holds the bitmaps.
    pub storage: &'a GuestMemory,
}

/// Hypercall 12: connects the port or irq outlet r6 to plug r5 of thread r4 of the logical PPE r3.
///
/// The answer clears r4 to r10. It carries the port or the outlet with
/// its connection and its plug, and the hypervisor keeps it. For a port
/// with no pending event, lv1 takes the clear path, and the answer
/// carries lv1's clear of the plug's bit; see [`raise::clear`].
///
/// A call that fails one of lv1's checks gets lv1's own r3, with r4 to
/// r10 cleared. See [`IRQ_PLUG_PPE_ID_R3`] for the checks and their
/// order. A plug that holds another port or outlet leaves the new plug in
/// the port or the outlet without a connection, as lv1 does.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the logical PPE id for `firmware`;
/// - r6 names no object that CellGov holds;
/// - the port holds a pending event, for which lv1 raises the plug's
///   bit;
/// - no storage holds the status doubleword of the plug.
pub(crate) fn connect_plug(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &PlugState<'_>,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| irq_plug_handler(fw).is_some()) else {
        return refuse(
            request,
            "an IRQ plug call at a firmware whose lv1 handler is not read",
        );
    };
    let Some(logical_ppe_id) = game_os_logical_ppe_id(firmware) else {
        return refuse(
            request,
            "an IRQ plug call at a firmware whose logical PPE id no source gives",
        );
    };
    let (ppe_id, thread, plug, port_id) = (request.r(3), request.r(4), request.r(5), request.r(6));
    if ppe_id != logical_ppe_id {
        return lv1_answer(IRQ_PLUG_PPE_ID_R3);
    }
    let Some(thread) = u8::try_from(thread)
        .ok()
        .filter(|&t| u64::from(t) < THREADS_PER_LOGICAL_PPE)
    else {
        return lv1_answer(IRQ_PLUG_THREAD_R3);
    };
    let Some(plug) = u8::try_from(plug)
        .ok()
        .filter(|&p| u64::from(p) < IRQ_PLUGS_PER_THREAD)
    else {
        return lv1_answer(IRQ_PLUG_PLUG_R3);
    };
    if request.r(7) != 0 {
        return lv1_answer(IRQ_PLUG_R7_R3);
    }
    let (port, change): (EventReceivePort, fn(EventReceivePort) -> InterruptChange) =
        if let Some(port) = state.ports.port(port_id) {
            (*port, InterruptChange::PlugConnect)
        } else if let Some(outlet) = state.ports.outlet(port_id) {
            (*outlet, InterruptChange::OutletPlugConnect)
        } else if port_id == logical_ppe_id
            || state.spaces.space(port_id).is_some()
            || state.spes.spe(port_id).is_some()
        {
            return lv1_answer(IRQ_PLUG_PORT_R3);
        } else {
            return refuse(
                request,
                "an IRQ plug call whose port id names no object CellGov holds",
            );
        };
    if port.connection.is_some() {
        return lv1_answer(IRQ_PLUG_PORT_CONNECTED_R3);
    }
    let key = IrqStateBitmapKey { ppe_id, thread };
    if state.ports.port_on_plug(key, plug).is_some() {
        let mut answer = HypercallAnswer::lv1_status(IRQ_PLUG_IN_USE_R3, []);
        answer.push_change(change(EventReceivePort { plug, ..port }));
        return HypercallDisposition::Answered(answer);
    }
    if state.ports.is_pending(port.id) {
        return refuse(
            request,
            "an IRQ plug call for a port with a pending event, which lv1 raises on the plug",
        );
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    if let Err(error) = raise::clear(
        request,
        state.bitmaps,
        state.storage,
        key,
        plug,
        &mut answer,
    ) {
        return error.refusal(request);
    }
    answer.push_change(change(EventReceivePort {
        connection: Some(key),
        plug,
        ..port
    }));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/irq_plug_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/irq_plug_state_tests.rs"]
mod state_tests;

#[cfg(test)]
#[path = "tests/console_irq_plug_map_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/irq_plug_outlet_tests.rs"]
mod outlet_tests;

#[cfg(test)]
#[path = "tests/irq_plug_boot_outlet_tests.rs"]
mod boot_outlet_tests;

#[cfg(test)]
#[path = "tests/irq_plug_clear_path_tests.rs"]
mod clear_path_tests;

#[cfg(test)]
#[path = "tests/irq_plug_raised_bit_tests.rs"]
mod raised_bit_tests;
