//! Hypercall 24, `send_event_locally`: the call raises an event on an
//! event receive port.
//!
//! lv1 sets the port's pending byte. For a connected port, it raises the
//! plug's bit in the status shadow of the plug's thread. When that thread
//! is in a pause and its mask holds the bit, lv1 wakes the thread; see
//! [`cellgov_ps3_abi::lv1::send_event`]. The hypervisor keeps the pending
//! ports and the shadows, and hashes them. The event itself is the
//! interrupt group's [`raise::event`].
//!
//! The handler reads [`HypercallRequest::thread`] to tell a plug of the
//! caller from a plug of another thread, as lv1 does before its
//! interprocessor interrupt.

use cellgov_ps3_abi::lv1::call::SEND_EVENT_LOCALLY;
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_ps3_abi::lv1::send_event::{send_event_handler, SEND_EVENT_NO_ENTRY_R3};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::raise::{self, RaiseState};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::group::memory::address_space::AddressSpaceState;
use crate::group::spe::LogicalSpeState;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: SEND_EVENT_LOCALLY,
    name: "send_event_locally",
    cost: None,
    handler: |request, input| {
        let interrupt = input.groups.interrupt();
        send_event(
            request,
            input.firmware,
            &SendState {
                spaces: input.groups.memory().spaces(),
                spes: input.groups.spe().spes(),
                raise: RaiseState {
                    bitmaps: interrupt.bitmaps(),
                    ports: interrupt.ports(),
                    paused: interrupt.paused(),
                    storage: input.storage,
                },
            },
        )
    },
}];

/// The state that the hypervisor holds for an event send.
pub(crate) struct SendState<'a> {
    /// The virtual address spaces, whose ids name objects of another
    /// class.
    pub spaces: &'a AddressSpaceState,
    /// The logical SPEs, whose ids name objects of another class.
    pub spes: &'a LogicalSpeState,
    /// The interrupt state that the event reads.
    pub raise: RaiseState<'a>,
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

/// Hypercall 24: raises an event on the port r3.
///
/// The answer is r3 = 0 with r4 to r10 cleared, for each of these ports:
/// - A port with a pending event: lv1 changes nothing.
/// - A port with no connection: the answer carries the pending event.
/// - A connected port whose plug bit is set in the thread's shadow: the
///   answer carries the pending event.
/// - A connected port of the calling thread or of a paused thread: the
///   answer carries the pending event and the raised shadow doubleword.
///   It writes that doubleword to the thread's bitmap. For a paused
///   thread whose mask holds the bit, it carries lv1's wake of the
///   thread too.
///
/// The answer is [`SEND_EVENT_NO_ENTRY_R3`] with r4 to r10 cleared when r3
/// names one of these objects:
/// - an irq outlet;
/// - the logical PPE;
/// - an address space;
/// - a logical SPE.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 names no object that CellGov holds;
/// - the raise is a refusal of [`raise::raise`].
pub(crate) fn send_event(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &SendState<'_>,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| send_event_handler(fw).is_some()) else {
        return refuse(
            request,
            "an event send at a firmware whose lv1 handler is not read",
        );
    };
    let id = request.r(3);
    let ports = state.raise.ports;
    if ports.port(id).is_none() {
        let other_object = ports.outlet(id).is_some()
            || game_os_logical_ppe_id(firmware) == Some(id)
            || state.spaces.space(id).is_some()
            || state.spes.spe(id).is_some();
        if other_object {
            return lv1_answer(SEND_EVENT_NO_ENTRY_R3);
        }
        return refuse(
            request,
            "an event send whose port id names no object CellGov holds",
        );
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    if let Err(error) = raise::event(request, &state.raise, id, &mut answer) {
        return error.refusal(request);
    }
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/send_event_tests.rs"]
mod tests;
