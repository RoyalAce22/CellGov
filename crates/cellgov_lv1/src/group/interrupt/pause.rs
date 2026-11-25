//! Hypercall 9, `pause`: lv1 pauses the calling thread until an
//! interrupt wakes it. The hypervisor keeps each paused thread and its
//! decrementer wake flag.
//!
//! lv1 pauses the thread that makes the call; see
//! [`cellgov_ps3_abi::lv1::pause`]. So the handler reads
//! [`HypercallRequest::thread`].
//!
//! A raise on an unmasked plug of a paused thread wakes it; see
//! [`super::raise::raise`]. The wake ends the pause, and the thread
//! returns from the call through lv1's vector, which checks the thread's
//! status shadow against its mask; see
//! [`cellgov_ps3_abi::lv1::interrupt_mask`]. In mode 0, the thread's
//! decrementer also wakes it; see [`crate::DecrementerWake`].

use std::collections::BTreeMap;

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::PAUSE;
use cellgov_ps3_abi::lv1::pause::{
    pause_handler, PAUSE_MODE_DECREMENTER_IGNORED, PAUSE_MODE_DECREMENTER_WAKES, PAUSE_MODE_R3,
};
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::bitmap::{IrqStateBitmapKey, IrqStateBitmapState};
use super::raise::unmasked;
use super::{InterruptChange, PAUSED_THREAD_SOURCE};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: PAUSE,
    name: "pause",
    cost: None,
    handler: |request, input| {
        pause(
            request,
            input.firmware,
            input.groups.interrupt().bitmaps(),
            input.storage,
        )
    },
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

/// The threads that lv1 paused, each with its wake flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PausedThreadState {
    paused: BTreeMap<u8, bool>,
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

    /// Ends the pause of the thread `thread`, as lv1's wake clears the
    /// thread's pause flag.
    pub(crate) fn wake(&mut self, thread: u8) {
        self.paused.remove(&thread);
    }

    /// The sync-state terms of the paused threads.
    ///
    /// Each paused thread is one object of the lane source
    /// `PAUSED_THREAD_SOURCE`, keyed by its index in thread order. A
    /// state with no paused thread adds no term.
    pub fn sync_term(&self) -> u128 {
        (0u64..)
            .zip(self.threads())
            .fold(0u128, |sum, (index, paused)| {
                sum.wrapping_add(lanes::value_term(PAUSED_THREAD_SOURCE, index, &paused))
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
/// For mode 0 or 1, the answer is r3 = 0 with r4 to r10 cleared. lv1
/// ends the call with no pause in these cases:
/// - the caller's status shadow holds a bit that its mask holds
///   [OFW-4.93 lv1 a:0x2C6930-0x2C695C];
/// - in mode 0, a decrementer interrupt is pending in the byte at +1830.
///   CellGov's hypervisor keeps no such byte.
///
/// Otherwise the caller pauses: the answer carries the paused thread and
/// its wake flag, and the hypervisor keeps them.
///
/// Any other mode gets [`PAUSE_MODE_R3`] with r4 to r10 cleared, and the
/// caller does not pause.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - the request names no hardware thread;
/// - the test of the shadow against the mask is a refusal of
///   [`unmasked`].
pub(crate) fn pause(
    request: &HypercallRequest,
    firmware: Option<&str>,
    bitmaps: &IrqStateBitmapState,
    storage: &GuestMemory,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| pause_handler(fw).is_some()) else {
        return refuse(
            request,
            "a pause at a firmware whose lv1 handler is not read",
        );
    };
    let decrementer_wakes = match request.r(3) {
        PAUSE_MODE_DECREMENTER_WAKES => true,
        PAUSE_MODE_DECREMENTER_IGNORED => false,
        _ => return HypercallDisposition::Answered(HypercallAnswer::lv1_status(PAUSE_MODE_R3, [])),
    };
    let Some(thread) = request.thread else {
        return refuse(request, "a pause from a unit that names no hardware thread");
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    let pending = match game_os_logical_ppe_id(firmware) {
        Some(ppe_id) => unmasked(bitmaps, IrqStateBitmapKey { ppe_id, thread }, storage),
        None => Ok(false),
    };
    match pending {
        Ok(true) => return HypercallDisposition::Answered(answer),
        Ok(false) => {}
        Err(error) => return error.refusal(request),
    }
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
