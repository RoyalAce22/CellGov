//! Hypercall 9, `pause`: lv1 pauses the calling thread until an
//! interrupt wakes it. The hypervisor keeps each paused thread and its
//! decrementer wake flag.
//!
//! lv1 pauses the thread that makes the call; see
//! [`cellgov_ps3_abi::lv1::pause`]. So the handler reads
//! [`HypercallRequest::thread`].
//!
//! CellGov delivers no wake to a paused thread:
//! - `send_event_locally` can raise an unmasked plug's bit on a paused
//!   thread. lv1 then clears the thread's pause flag, and the hypervisor
//!   keeps that wake; see [`PausedThreadState::is_woken`].
//! - CellGov does not model the decrementer wake of mode 0.
//!
//! A paused thread therefore stays parked, and the run that holds it says
//! so where it stops. A thread that resumes from a pause returns through
//! lv1's vector, which checks the thread's status shadow against its mask;
//! see [`cellgov_ps3_abi::lv1::interrupt_mask`].

use std::collections::{BTreeMap, BTreeSet};

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::PAUSE;
use cellgov_ps3_abi::lv1::pause::{
    pause_handler, PAUSE_MODE_DECREMENTER_IGNORED, PAUSE_MODE_DECREMENTER_WAKES, PAUSE_MODE_R3,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{InterruptChange, PAUSED_THREAD_SOURCE, WOKEN_THREAD_SOURCE};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: PAUSE,
    name: "pause",
    cost: None,
    handler: |request, input| pause(request, input.firmware),
}];

/// One thread that lv1 paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PausedThread {
    /// The PPE hardware thread that made the call.
    pub thread: u8,
    /// Whether a decrementer interrupt wakes the thread: lv1's wake flag,
    /// set for mode 0.
    pub decrementer_wakes: bool,
}

/// Fields 1 and 2: the thread and the wake flag.
impl LaneValue for PausedThread {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.thread));
        lanes.lane(2, 0, u64::from(self.decrementer_wakes));
    }
}

/// A paused thread that lv1 woke. Field 1 is the thread.
struct WokenThread(u8);

impl LaneValue for WokenThread {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.0));
    }
}

/// The threads that lv1 paused, each with its wake flag, and the paused
/// threads that lv1 woke.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PausedThreadState {
    paused: BTreeMap<u8, bool>,
    woken: BTreeSet<u8>,
}

impl PausedThreadState {
    /// Each paused thread, in thread order.
    pub fn threads(&self) -> impl Iterator<Item = PausedThread> + '_ {
        self.paused
            .iter()
            .map(|(&thread, &decrementer_wakes)| PausedThread {
                thread,
                decrementer_wakes,
            })
    }

    /// The paused thread `thread`, or `None` when that thread is not
    /// paused.
    pub fn thread(&self, thread: u8) -> Option<PausedThread> {
        self.paused
            .get(&thread)
            .map(|&decrementer_wakes| PausedThread {
                thread,
                decrementer_wakes,
            })
    }

    /// Keeps `paused`.
    pub(crate) fn pause(&mut self, paused: PausedThread) {
        self.paused.insert(paused.thread, paused.decrementer_wakes);
    }

    /// Whether lv1 cleared the pause flag of the paused thread `thread`.
    ///
    /// lv1 clears the flag for an interrupt on an unmasked plug. CellGov
    /// delivers no wake, so the thread stays parked.
    pub fn is_woken(&self, thread: u8) -> bool {
        self.woken.contains(&thread)
    }

    /// Keeps the wake of the paused thread `thread`.
    pub(crate) fn wake(&mut self, thread: u8) {
        self.woken.insert(thread);
    }

    /// The sync-state terms of the paused threads and their wakes.
    ///
    /// Each paused thread is one object of the lane source
    /// `PAUSED_THREAD_SOURCE`, and each woken thread is one object of the
    /// lane source `WOKEN_THREAD_SOURCE`. Each object's key is its index
    /// in thread order. A state with no paused thread adds no term.
    pub fn sync_term(&self) -> u128 {
        let paused = (0u64..)
            .zip(self.threads())
            .fold(0u128, |sum, (index, paused)| {
                sum.wrapping_add(lanes::value_term(PAUSED_THREAD_SOURCE, index, &paused))
            });
        (0u64..)
            .zip(&self.woken)
            .fold(paused, |sum, (index, &thread)| {
                sum.wrapping_add(lanes::value_term(
                    WOKEN_THREAD_SOURCE,
                    index,
                    &WokenThread(thread),
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

/// Hypercall 9: pauses the calling thread in the mode r3.
///
/// For mode 0 or 1, the answer is r3 = 0 with r4 to r10 cleared, and
/// the caller pauses. The answer carries the paused thread and its wake
/// flag, and the hypervisor keeps them. lv1 ends the call with no pause
/// in two cases, and CellGov's hypervisor answers neither:
/// - an unmasked status shadow bit. The hypervisor raises a bit only on
///   a paused thread, so the caller's shadow is 0;
/// - in mode 0, a pending decrementer interrupt that lv1 keeps in the
///   byte at +1830. CellGov's hypervisor keeps no such byte.
///
/// Any other mode gets [`PAUSE_MODE_R3`] with r4 to r10 cleared, and the
/// caller does not pause.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - the request names no hardware thread.
pub(crate) fn pause(request: &HypercallRequest, firmware: Option<&str>) -> HypercallDisposition {
    if firmware.and_then(pause_handler).is_none() {
        return refuse(
            request,
            "a pause at a firmware whose lv1 handler is not read",
        );
    }
    let decrementer_wakes = match request.r(3) {
        PAUSE_MODE_DECREMENTER_WAKES => true,
        PAUSE_MODE_DECREMENTER_IGNORED => false,
        _ => return HypercallDisposition::Answered(HypercallAnswer::lv1_status(PAUSE_MODE_R3, [])),
    };
    let Some(thread) = request.thread else {
        return refuse(request, "a pause from a unit that names no hardware thread");
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(InterruptChange::ThreadPaused(PausedThread {
        thread,
        decrementer_wakes,
    }));
    answer.pauses_caller = true;
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/pause_tests.rs"]
mod tests;
