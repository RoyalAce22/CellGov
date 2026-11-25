//! Hypercall 31, `did_update_interrupt_mask`: the call tells lv1 that the
//! mask doublewords of a thread's interrupt state bitmap changed.
//!
//! lv1 compares r4 with the calling thread; see
//! [`cellgov_ps3_abi::lv1::interrupt_mask`]. So the handler reads
//! [`HypercallRequest::thread`].

use cellgov_ps3_abi::lv1::call::DID_UPDATE_INTERRUPT_MASK;
use cellgov_ps3_abi::lv1::interrupt::THREADS_PER_LOGICAL_PPE;
use cellgov_ps3_abi::lv1::interrupt_mask::{
    interrupt_mask_handler, INTERRUPT_MASK_PPE_ID_R3, INTERRUPT_MASK_THREAD_R3,
};
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: DID_UPDATE_INTERRUPT_MASK,
    name: "did_update_interrupt_mask",
    cost: None,
    handler: |request, input| update_mask(request, input.firmware),
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

/// Hypercall 31: the mask of thread r4 of the logical PPE r3 changed.
///
/// For the calling thread, the answer is r3 = 0 with r4 to r10 cleared,
/// and no change. lv1's return path then raises the external interrupt
/// only for a set bit of the thread's status shadow. The shadow is 0,
/// because no call that CellGov answers raises a plug's bit.
///
/// A call that fails one of lv1's checks gets lv1's own r3, with r4 to
/// r10 cleared. See [`INTERRUPT_MASK_PPE_ID_R3`] for the checks and their
/// order.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the logical PPE id for `firmware`;
/// - the request names no hardware thread;
/// - r4 names the other thread, which lv1 signals with an interprocessor
///   interrupt that CellGov does not deliver.
pub(crate) fn update_mask(
    request: &HypercallRequest,
    firmware: Option<&str>,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| interrupt_mask_handler(fw).is_some()) else {
        return refuse(
            request,
            "an interrupt mask update at a firmware whose lv1 handler is not read",
        );
    };
    let Some(logical_ppe_id) = game_os_logical_ppe_id(firmware) else {
        return refuse(
            request,
            "an interrupt mask update at a firmware whose logical PPE id no source gives",
        );
    };
    if request.r(3) != logical_ppe_id {
        return lv1_answer(INTERRUPT_MASK_PPE_ID_R3);
    }
    let Some(thread) = u8::try_from(request.r(4))
        .ok()
        .filter(|&t| u64::from(t) < THREADS_PER_LOGICAL_PPE)
    else {
        return lv1_answer(INTERRUPT_MASK_THREAD_R3);
    };
    match request.thread {
        None => refuse(
            request,
            "an interrupt mask update from a unit that names no hardware thread",
        ),
        Some(caller) if caller == thread => lv1_answer(zero_extended(LV1_SUCCESS)),
        Some(_) => refuse(
            request,
            "an interrupt mask update for the other hardware thread, which lv1 signals with an interprocessor interrupt",
        ),
    }
}

#[cfg(test)]
#[path = "tests/mask_tests.rs"]
mod tests;
