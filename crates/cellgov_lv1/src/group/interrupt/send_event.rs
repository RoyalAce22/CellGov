//! Hypercall 24, `send_event_locally`: the call raises an event on an
//! event receive port.
//!
//! lv1 sets the port's pending byte. For a connected port, it raises the
//! plug's bit in the status shadow of the plug's thread. It wakes that
//! thread when the thread is paused and the bit is unmasked; see
//! [`cellgov_ps3_abi::lv1::send_event`]. The hypervisor keeps the pending
//! ports, the shadows and the wakes, and hashes them.
//!
//! CellGov delivers no external interrupt and no wake. lv1 checks a raised
//! bit against the thread's mask at each hypercall return of that thread,
//! and CellGov's dispatch does not. So the hypervisor raises a bit only on
//! a paused thread, which makes no call. A paused thread that lv1 wakes
//! stays parked.
//!
//! The handler reads [`HypercallRequest::thread`] to tell a plug of the
//! caller from a plug of another thread, as lv1 does before its
//! interprocessor interrupt.

use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::{be::read_u64, ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::call::SEND_EVENT_LOCALLY;
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;
use cellgov_ps3_abi::lv1::irq_plug::plug_status_bit;
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_ps3_abi::lv1::send_event::{send_event_handler, SEND_EVENT_NO_ENTRY_R3};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::bitmap::{IrqStateBitmapKey, IrqStateBitmapState, StatusShadowWrite};
use super::event_port::EventReceivePortState;
use super::pause::PausedThreadState;
use super::InterruptChange;
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
                bitmaps: interrupt.bitmaps(),
                ports: interrupt.ports(),
                paused: interrupt.paused(),
                storage: input.storage,
            },
        )
    },
}];

/// The bytes of one status or mask doubleword.
const DOUBLEWORD_BYTES: u64 = std::mem::size_of::<u64>() as u64;

/// The state that the hypervisor holds for an event send.
pub(crate) struct SendState<'a> {
    /// The virtual address spaces, whose ids name objects of another
    /// class.
    pub spaces: &'a AddressSpaceState,
    /// The logical SPEs, whose ids name objects of another class.
    pub spes: &'a LogicalSpeState,
    /// The interrupt state bitmaps and the status shadows.
    pub bitmaps: &'a IrqStateBitmapState,
    /// The event receive ports, the irq outlets and the pending events.
    pub ports: &'a EventReceivePortState,
    /// The paused threads and their wakes.
    pub paused: &'a PausedThreadState,
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

/// Hypercall 24: raises an event on the port r3.
///
/// The answer is r3 = 0 with r4 to r10 cleared, for each of these ports:
/// - A port with a pending event: lv1 changes nothing.
/// - A port with no connection: the answer carries the pending event.
/// - A connected port whose plug bit is set in the thread's shadow: the
///   answer carries the pending event.
/// - A connected port of a paused thread: the answer carries the pending
///   event and the raised shadow doubleword. It writes that doubleword to
///   the thread's bitmap. For a plug bit that the bitmap's mask holds, it
///   carries lv1's wake of the thread too, unless lv1 already woke it.
///
/// An r3 that names an irq outlet, the logical PPE, an address space or a
/// logical SPE gets [`SEND_EVENT_NO_ENTRY_R3`] with r4 to r10 cleared.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 names no object that CellGov holds;
/// - the raise is for a plug of the calling thread, whose return lv1
///   checks for an external interrupt;
/// - the raise is for a plug of a running thread, which the
///   interprocessor interrupt from lv1 interrupts (lv1 sends that
///   interrupt to a paused thread on another CPU too, at 0x2C663C);
/// - the raise is for a thread with no registered bitmap;
/// - no storage holds the plug's status and mask doublewords.
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
    let Some(port) = state.ports.port(id) else {
        let other_object = state.ports.outlet(id).is_some()
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
    };
    if state.ports.is_pending(id) {
        return lv1_answer(zero_extended(LV1_SUCCESS));
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(InterruptChange::EventPending(id));
    let Some(key) = port.connection else {
        return HypercallDisposition::Answered(answer);
    };
    let (doubleword, bit) = plug_status_bit(port.plug);
    let index = doubleword as u8;
    let shadow = state.bitmaps.shadow(key)[usize::from(index)];
    if shadow & bit != 0 {
        return HypercallDisposition::Answered(answer);
    }
    if let Err(what) = raise(request, state, key, index, shadow | bit, &mut answer) {
        return refuse(request, what);
    }
    HypercallDisposition::Answered(answer)
}

/// Adds lv1's raise of the plug bit `value ^ shadow` of the thread `key`
/// to `answer`, or gives the input that CellGov does not model.
///
/// `value` is the status doubleword `index` with the bit set.
fn raise(
    request: &HypercallRequest,
    state: &SendState<'_>,
    key: IrqStateBitmapKey,
    index: u8,
    value: u64,
    answer: &mut HypercallAnswer,
) -> Result<(), &'static str> {
    if request.thread == Some(key.thread) {
        return Err(
            "an event on a plug of the calling thread, whose return lv1 checks for an external interrupt",
        );
    }
    if state.paused.thread(key.thread).is_none() {
        return Err(
            "an event on a plug of a running thread, which lv1 signals with an interprocessor interrupt",
        );
    }
    let Some(bitmap) = state.bitmaps.bitmap(key) else {
        return Err("an event on a plug of a thread that registered no interrupt state bitmap");
    };
    let doubleword = |offset: u64| {
        bitmap
            .checked_add(offset + u64::from(index) * DOUBLEWORD_BYTES)
            .and_then(|address| ByteRange::new(GuestAddr::new(address), DOUBLEWORD_BYTES))
    };
    let status =
        doubleword(0).filter(|range| state.storage.holds(range.start().raw(), range.length()));
    let mask = doubleword(IRQ_MASK_OFFSET).and_then(|range| state.storage.read(range));
    let (Some(status), Some(mask)) = (status, mask) else {
        return Err(
            "an event on a plug whose status and mask doublewords CellGov holds no storage for",
        );
    };
    let mask = read_u64(&mask, 0);
    answer.effects.push(Effect::SharedWriteIntent {
        range: status,
        bytes: WritePayload::new(value.to_be_bytes().to_vec()),
        source: request.unit,
        source_time: request.tick,
    });
    answer.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
        key,
        index,
        value,
    }));
    let bit = value & !state.bitmaps.shadow(key)[usize::from(index)];
    if mask & bit != 0 && !state.paused.is_woken(key.thread) {
        answer.push_change(InterruptChange::ThreadWoken(key.thread));
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/send_event_tests.rs"]
mod tests;
