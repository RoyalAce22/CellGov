//! Hypercall 30, `end_of_interrupt_ext`: the call ends the interrupt of
//! the port or irq outlet on a plug of a logical PPE thread.
//!
//! lv1 clears the object's pending byte and the plug's bit in the
//! thread's status shadow. It then runs the object's node list. The node
//! of the virtual UART's outlet gives the outlet another event while the
//! UART's shadow holds a bit; see
//! [`cellgov_ps3_abi::lv1::end_of_interrupt`].
//!
//! The handler reads [`HypercallRequest::thread`] for that event, as
//! lv1's raise does before its interprocessor interrupt.

use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::END_OF_INTERRUPT_EXT;
use cellgov_ps3_abi::lv1::end_of_interrupt::{
    end_of_interrupt_handler, END_OF_INTERRUPT_NOT_PENDING_R3, END_OF_INTERRUPT_PLUG_R3,
    END_OF_INTERRUPT_PPE_ID_R3, END_OF_INTERRUPT_THREAD_R3, END_OF_INTERRUPT_WRONG_STATE_R3,
};
use cellgov_ps3_abi::lv1::interrupt::THREADS_PER_LOGICAL_PPE;
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::bitmap::IrqStateBitmapKey;
use super::raise::{self, RaiseState};
use super::{InterruptChange, InterruptGroupState};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::group::uart::{UartState, UART_SHADOW_DOUBLEWORDS};
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: END_OF_INTERRUPT_EXT,
    name: "end_of_interrupt_ext",
    cost: None,
    handler: |request, input| {
        end_of_interrupt(
            request,
            input.firmware,
            &EndState {
                interrupt: input.groups.interrupt(),
                uart: input.groups.uart(),
                storage: input.storage,
            },
        )
    },
}];

/// The state that the hypervisor holds for an end of interrupt.
pub(crate) struct EndState<'a> {
    /// The bitmaps, the shadows, the ports, the irq outlets and the
    /// paused threads.
    pub interrupt: &'a InterruptGroupState,
    /// The virtual UART's outlet and its shadow.
    pub uart: &'a UartState,
    /// The caller's real address space, which holds the bitmaps.
    pub storage: &'a GuestMemory,
}

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

/// Hypercall 30: ends the interrupt on plug r5 of thread r4 of the logical PPE r3.
///
/// A call that fails one of lv1's checks gets lv1's own r3, with r4 to
/// r10 cleared:
/// - r3 is not the partition's logical PPE: [`END_OF_INTERRUPT_PPE_ID_R3`];
/// - r4 names no thread: [`END_OF_INTERRUPT_THREAD_R3`];
/// - r5 is above 0xFF: [`END_OF_INTERRUPT_PLUG_R3`];
/// - the plug holds no port or irq outlet:
///   [`END_OF_INTERRUPT_WRONG_STATE_R3`];
/// - the object holds no pending event: [`END_OF_INTERRUPT_NOT_PENDING_R3`].
///
/// Otherwise the answer is r3 = 0 with r4 to r10 cleared. It carries
/// these changes in lv1's order:
/// - the cleared pending event;
/// - lv1's clear of the plug's bit; see [`raise::clear`];
/// - for the virtual UART's outlet while the UART's shadow holds a bit,
///   the outlet's event; see [`raise::event`]. The event reads the state
///   after the clear.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the logical PPE id for `firmware`;
/// - the plug holds an irq outlet of a logical SPE, whose node list lv1
///   runs and CellGov does not model;
/// - the clear or the event is a refusal.
pub(crate) fn end_of_interrupt(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &EndState<'_>,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| end_of_interrupt_handler(fw).is_some()) else {
        return refuse(
            request,
            "an end of interrupt at a firmware whose lv1 handler is not read",
        );
    };
    let Some(logical_ppe_id) = game_os_logical_ppe_id(firmware) else {
        return refuse(
            request,
            "an end of interrupt at a firmware whose logical PPE id no source gives",
        );
    };
    if request.r(3) != logical_ppe_id {
        return lv1_answer(END_OF_INTERRUPT_PPE_ID_R3);
    }
    let Some(thread) = u8::try_from(request.r(4))
        .ok()
        .filter(|&t| u64::from(t) < THREADS_PER_LOGICAL_PPE)
    else {
        return lv1_answer(END_OF_INTERRUPT_THREAD_R3);
    };
    let Ok(plug) = u8::try_from(request.r(5)) else {
        return lv1_answer(END_OF_INTERRUPT_PLUG_R3);
    };
    let key = IrqStateBitmapKey {
        ppe_id: logical_ppe_id,
        thread,
    };
    let ports = state.interrupt.ports();
    let Some(id) = ports.port_on_plug(key, plug).map(|port| port.id) else {
        return lv1_answer(END_OF_INTERRUPT_WRONG_STATE_R3);
    };
    if !ports.is_pending(id) {
        return lv1_answer(END_OF_INTERRUPT_NOT_PENDING_R3);
    }
    let Some(raises_again) = node_list_raise(state, id) else {
        return refuse(
            request,
            "an end of interrupt on an irq outlet of a logical SPE, whose node list lv1 runs",
        );
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(InterruptChange::EventCleared(id));
    let bitmaps = state.interrupt.bitmaps();
    if let Err(error) = raise::clear(request, bitmaps, state.storage, key, plug, &mut answer) {
        return error.refusal(request);
    }
    if raises_again {
        let after = state.interrupt.after(&answer.changes);
        let raise_state = RaiseState {
            bitmaps: after.bitmaps(),
            ports: after.ports(),
            paused: after.paused(),
            storage: state.storage,
        };
        if let Err(error) = raise::event(request, &raise_state, id, &mut answer) {
            return error.refusal(request);
        }
    }
    HypercallDisposition::Answered(answer)
}

/// Whether the node list of the port or irq outlet `id` gives the object
/// another event, or `None` when CellGov does not model the list.
///
/// The lists are these; see [`cellgov_ps3_abi::lv1::end_of_interrupt`]:
/// - a port of `construct_event_receive_port` has an empty list;
/// - the virtual UART's outlet has one node, which gives the outlet an
///   event while the UART's shadow holds a bit
///   [OFW-4.93 lv1 a:0x2F11D0-0x2F11E0];
/// - an irq outlet of a logical SPE has a node that CellGov does not
///   model.
fn node_list_raise(state: &EndState<'_>, id: u64) -> Option<bool> {
    if state.interrupt.ports().port(id).is_some() {
        return Some(false);
    }
    let uart_outlet = state.uart.irq().map(|irq| irq.outlet);
    (uart_outlet == Some(id)).then(|| state.uart.shadow() != [0; UART_SHADOW_DOUBLEWORDS])
}

#[cfg(test)]
#[path = "tests/end_of_interrupt_tests.rs"]
mod tests;
