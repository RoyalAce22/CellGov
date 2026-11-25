//! The decrementer wake of a thread that lv1 paused in mode 0.
//!
//! lv1 arms a timer for the paused thread as it saves the thread. The
//! deadline is the time base plus the decrementer, sign-extended
//! [OFW-4.93 lv1 a:0x2C6110-0x2C613C]. lv1 arms it only while the pause
//! flag and the wake flag are both set [OFW-4.93 lv1 a:0x2C6264-0x2C629C].
//! When the timer fires with the pause flag set, lv1 does these steps
//! under the PPE lock [OFW-4.93 lv1 a:0x2C5CC8-0x2C5D3C]:
//! - it sets the byte at +1830, the pending decrementer interrupt;
//! - it clears the pause flag;
//! - it runs the wake list.
//!
//! The thread then returns from `pause`. The handler returns r3 = 0
//! whatever ended the pause: it sets r31 to 0 before it calls the pause
//! step [OFW-4.93 lv1 a:0x2EB6F8], and returns r31 as r3
//! [OFW-4.93 lv1 a:0x2EB76C]. So the answer of the call, given when the
//! thread paused, is the answer of the wake.
//!
//! On the return, lv1 clears the byte at +1830 and writes DEC = 0
//! [OFW-4.93 lv1 a:0x2C425C-0x2C4274]. So the [`ThreadResume`] of the
//! wake sets [`ThreadResume::clears_decrementer`], and the thread reads
//! DEC = 0 at its resume. This holds also after a pause whose decrementer
//! was already negative.
//!
//! [CBE-Handbook p:321 s:10.8.2.1] A suspended PPE thread resumes on a
//! decrementer interrupt when TSCR enables that wake for the thread.

use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::PAUSE;

use crate::answer::{HypercallRefusal, ThreadResume};
use crate::external::wake_return;
use crate::group::state::GroupStates;
use crate::return_path::SavedMsrs;

/// The decrementer wake of one paused thread, as the runtime asks for it
/// at the thread's deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecrementerWake {
    /// The unit that runs the thread.
    pub unit: UnitId,
    /// The PPE hardware thread.
    pub thread: u8,
    /// The address of the `sc` of the thread's `pause`.
    pub pc: u64,
}

/// The [`ThreadResume`] of the thread of `wake`, or `None` when lv1
/// holds that thread in no pause that its decrementer ends.
///
/// # Errors
///
/// Returns [`HypercallRefusal::UnmodelledInput`] for the `pause` of the
/// thread when the test of its return refuses; see [`wake_return`].
pub(crate) fn wake_entry(
    wake: &DecrementerWake,
    firmware: Option<&str>,
    groups: &GroupStates,
    storage: &GuestMemory,
    msrs: &SavedMsrs,
) -> Result<Option<ThreadResume>, HypercallRefusal> {
    let paused = groups.interrupt().paused().thread(wake.thread);
    if !paused.is_some_and(|p| p.decrementer_wakes) {
        return Ok(None);
    }
    // lv1's timer sets the byte at +1830 as it ends the pause
    // [OFW-4.93 lv1 a:0x2C5D2C].
    wake_return(wake.thread, firmware, groups, storage, msrs)
        .map(|resume| {
            Some(ThreadResume {
                clears_decrementer: true,
                ..resume
            })
        })
        .map_err(|error| HypercallRefusal::UnmodelledInput {
            number: PAUSE,
            unit: wake.unit,
            pc: wake.pc,
            what: error.what,
        })
}

#[cfg(test)]
#[path = "tests/decrementer_wake_tests.rs"]
mod tests;
