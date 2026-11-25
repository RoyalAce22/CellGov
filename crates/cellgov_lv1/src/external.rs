//! The external interrupt of each return from lv1 that a call causes.
//!
//! A call returns to its caller, unless it pauses the caller. A call that
//! wakes a paused thread also makes that thread return from its `pause`.
//! Each return ANDs the thread's status shadow with its mask, and raises
//! the thread's external interrupt for a nonzero result; see
//! [`cellgov_ps3_abi::lv1::interrupt_mask`].
//!
//! A call that sends a running thread an interprocessor interrupt makes
//! that thread enter lv1's handler, which makes the same test; see
//! [`crate::interprocessor`].

use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;

use crate::answer::{
    HypercallAnswer, HypercallDisposition, ReturnPath, ThreadInterrupt, ThreadResume,
    UnmodelledError,
};
use crate::group::interrupt::bitmap::IrqStateBitmapKey;
use crate::group::interrupt::raise::unmasked;
use crate::group::interrupt::{InterruptChange, InterruptGroupState};
use crate::group::state::{GroupStates, StateChange};
use crate::interprocessor::{handler_raise, RunningThreads};
use crate::request::HypercallRequest;
use crate::return_path::{external_raise, ExternalRaise, SavedMsrs};

/// `answer` with the external interrupt of each return that it causes,
/// or the refusal of `request` when CellGov does not model one of them.
///
/// The test reads the interrupt state that `groups` holds after the
/// answer's changes, and the mask doublewords in `storage`. It covers
/// these returns:
/// - the caller's return, for an answer through `hrfid` that does not
///   pause the caller [OFW-4.93 lv1 a:0x0FC4-0x101C];
/// - the return of each thread that the answer wakes. The thread returns
///   from `pause` through the same vector.
///
/// It also gives the external interrupt of lv1's handler for each running
/// thread that the answer signals. The handler reads the registers that
/// `running` gives; see [`handler_raise`].
///
/// The `rfid` return for a number past the table makes no test
/// [OFW-4.93 lv1 a:0x0C44-0x0C58]. A firmware whose logical PPE id no
/// table gives makes no test either.
///
/// The disposition is a refusal in these cases:
/// - the shadow test refuses; see [`unmasked`];
/// - a return raises the interrupt, and `msrs` gives no saved MSR for its
///   thread;
/// - a return raises the interrupt, and [`external_raise`] refuses;
/// - [`handler_raise`] refuses for a signalled thread;
/// - the answer signals a thread, and the call makes no test.
pub(crate) fn returns(
    request: &HypercallRequest,
    firmware: Option<&str>,
    groups: &GroupStates,
    storage: &GuestMemory,
    msrs: &SavedMsrs,
    running: &RunningThreads,
    mut answer: HypercallAnswer,
) -> HypercallDisposition {
    // The IIC merges a second interrupt of the same priority into the
    // pending one [CBE-Handbook p:268 s:9.6.2.4], so a thread takes one.
    let mut signalled: Vec<u8> = answer.changes.iter().filter_map(signalled_thread).collect();
    signalled.sort_unstable();
    signalled.dedup();
    let tested = matches!(answer.path, ReturnPath::Table(_));
    let Some(ppe_id) = firmware.and_then(game_os_logical_ppe_id).filter(|_| tested) else {
        if signalled.is_empty() {
            return HypercallDisposition::Answered(answer);
        }
        return UnmodelledError::new(
            "an interprocessor interrupt from a call whose return makes no shadow test",
        )
        .refusal(request);
    };
    let test = ReturnTest {
        after: groups.interrupt().after(&answer.changes),
        ppe_id,
        storage,
        msrs,
        running,
    };
    let caller = request.thread.filter(|_| !answer.pauses_caller);
    let woken: Vec<u8> = answer.changes.iter().filter_map(woken_thread).collect();
    let deliveries = test
        .deliveries(caller, &woken)
        .and_then(|delivered| Ok((delivered, test.interrupts(&signalled)?)));
    match deliveries {
        Ok(((external, resumed), interrupted)) => {
            answer.external = external;
            answer.resumed = resumed;
            answer.interrupted = interrupted;
            HypercallDisposition::Answered(answer)
        }
        Err(error) => error.refusal(request),
    }
}

/// The [`ThreadResume`] of the paused thread `thread` that a wake with no
/// call ends.
///
/// The thread returns from `pause` through the vector of the call, so its
/// return makes the shadow test of [`returns`]. The test reads the state
/// that `groups` holds after the wake. A firmware whose logical PPE id no
/// table gives makes no test. The resume writes no DEC; the caller sets
/// [`ThreadResume::clears_decrementer`] for a wake that sets lv1's byte.
///
/// # Errors
///
/// Returns the case that the model does not cover, as [`returns`] refuses
/// it for a woken thread.
pub(crate) fn wake_return(
    thread: u8,
    firmware: Option<&str>,
    groups: &GroupStates,
    storage: &GuestMemory,
    msrs: &SavedMsrs,
) -> Result<ThreadResume, UnmodelledError> {
    let Some(ppe_id) = firmware.and_then(game_os_logical_ppe_id) else {
        return Ok(ThreadResume {
            thread,
            external: None,
            clears_decrementer: false,
        });
    };
    let woken: StateChange = InterruptChange::ThreadWoken(thread).into();
    let running = RunningThreads::default();
    let test = ReturnTest {
        after: groups.interrupt().after(std::slice::from_ref(&woken)),
        ppe_id,
        storage,
        msrs,
        running: &running,
    };
    let external = test.delivery(thread)?;
    Ok(ThreadResume {
        thread,
        external,
        clears_decrementer: false,
    })
}

/// What the test of each return reads.
struct ReturnTest<'a> {
    /// The interrupt state after the answer's changes.
    after: InterruptGroupState,
    /// The logical PPE id of the partition.
    ppe_id: u64,
    /// The caller's real address space, which holds the masks.
    storage: &'a GuestMemory,
    /// The saved MSR of each thread.
    msrs: &'a SavedMsrs,
    /// The registers of each running thread.
    running: &'a RunningThreads,
}

impl ReturnTest<'_> {
    /// The external interrupt of the return of `thread`, or `None`.
    fn delivery(&self, thread: u8) -> Result<Option<ExternalRaise>, UnmodelledError> {
        let key = IrqStateBitmapKey {
            ppe_id: self.ppe_id,
            thread,
        };
        if !unmasked(self.after.bitmaps(), key, self.storage)? {
            return Ok(None);
        }
        let msr = self.msrs.of(thread).ok_or(UnmodelledError::new(
            "an external interrupt on the return to a thread whose unit gives no saved MSR",
        ))?;
        external_raise(msr).map(Some)
    }

    /// The interrupt of the return of `caller`, and the [`ThreadResume`]
    /// of each thread in `woken`.
    ///
    /// A raise that wakes a thread sets no byte at +1830, so its resume
    /// writes no DEC. CellGov does not model the byte's setter in lv1's
    /// common interrupt work [OFW-4.93 lv1 a:0x2C6374].
    fn deliveries(
        &self,
        caller: Option<u8>,
        woken: &[u8],
    ) -> Result<(Option<ExternalRaise>, Vec<ThreadResume>), UnmodelledError> {
        let external = match caller {
            Some(thread) => self.delivery(thread)?,
            None => None,
        };
        let resumed = woken
            .iter()
            .map(|&thread| {
                let external = self.delivery(thread)?;
                Ok(ThreadResume {
                    thread,
                    external,
                    clears_decrementer: false,
                })
            })
            .collect::<Result<Vec<_>, UnmodelledError>>()?;
        Ok((external, resumed))
    }

    /// The [`ThreadInterrupt`] of each running thread in `signalled`.
    fn interrupts(&self, signalled: &[u8]) -> Result<Vec<ThreadInterrupt>, UnmodelledError> {
        signalled
            .iter()
            .map(|&thread| {
                let key = IrqStateBitmapKey {
                    ppe_id: self.ppe_id,
                    thread,
                };
                let running = self.running.of(thread);
                let external = handler_raise(&self.after, key, self.storage, running)?;
                Ok(ThreadInterrupt { thread, external })
            })
            .collect()
    }
}

fn woken_thread(change: &StateChange) -> Option<u8> {
    match change {
        StateChange::Interrupt(change) => match **change {
            InterruptChange::ThreadWoken(thread) => Some(thread),
            _ => None,
        },
        _ => None,
    }
}

fn signalled_thread(change: &StateChange) -> Option<u8> {
    match change {
        StateChange::Interrupt(change) => match **change {
            InterruptChange::ThreadSignalled(thread) => Some(thread),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests/external_tests.rs"]
mod tests;
