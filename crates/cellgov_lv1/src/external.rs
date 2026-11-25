//! The external interrupt of each return from lv1 that a call causes.
//!
//! A call returns to its caller, unless it pauses the caller. A call that
//! wakes a paused thread also makes that thread return from its `pause`.
//! Each return ANDs the thread's status shadow with its mask, and raises
//! the thread's external interrupt for a nonzero result; see
//! [`cellgov_ps3_abi::lv1::interrupt_mask`].

use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;

use crate::answer::{
    HypercallAnswer, HypercallDisposition, ReturnPath, ThreadResume, UnmodelledError,
};
use crate::group::interrupt::bitmap::IrqStateBitmapKey;
use crate::group::interrupt::raise::unmasked;
use crate::group::interrupt::{InterruptChange, InterruptGroupState};
use crate::group::state::{GroupStates, StateChange};
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
/// The `rfid` return for a number past the table makes no test
/// [OFW-4.93 lv1 a:0x0C44-0x0C58]. A firmware whose logical PPE id no
/// table gives makes no test either.
///
/// The disposition is a refusal in these cases:
/// - the shadow test refuses; see [`unmasked`];
/// - a return raises the interrupt, and `msrs` gives no saved MSR for its
///   thread;
/// - a return raises the interrupt, and [`external_raise`] refuses.
pub(crate) fn returns(
    request: &HypercallRequest,
    firmware: Option<&str>,
    groups: &GroupStates,
    storage: &GuestMemory,
    msrs: &SavedMsrs,
    mut answer: HypercallAnswer,
) -> HypercallDisposition {
    let tested = matches!(answer.path, ReturnPath::Table(_));
    let Some(ppe_id) = firmware.and_then(game_os_logical_ppe_id).filter(|_| tested) else {
        return HypercallDisposition::Answered(answer);
    };
    let test = ReturnTest {
        after: groups.interrupt().after(&answer.changes),
        ppe_id,
        storage,
        msrs,
    };
    let caller = request.thread.filter(|_| !answer.pauses_caller);
    let woken: Vec<u8> = answer.changes.iter().filter_map(woken_thread).collect();
    match test.deliveries(caller, &woken) {
        Ok((external, resumed)) => {
            answer.external = external;
            answer.resumed = resumed;
            HypercallDisposition::Answered(answer)
        }
        Err(error) => error.refusal(request),
    }
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
                Ok(ThreadResume { thread, external })
            })
            .collect::<Result<Vec<_>, UnmodelledError>>()?;
        Ok((external, resumed))
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

#[cfg(test)]
#[path = "tests/external_tests.rs"]
mod tests;
