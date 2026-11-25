//! Hypercall 69, `get_logical_ppe_id`: the id of the caller's logical
//! PPE.
//!
//! lv1 reads the id from the logical PPE that the calling thread's lv1
//! context names. Both hardware threads of the GameOS partition run on
//! its one logical PPE, so the answer is the same for each thread, and
//! the handler does not read [`HypercallRequest::thread`].

use cellgov_ps3_abi::lv1::call::GET_LOGICAL_PPE_ID;
use cellgov_ps3_abi::lv1::ppe::{game_os_logical_ppe_id, logical_ppe_id_handler};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: GET_LOGICAL_PPE_ID,
    name: "get_logical_ppe_id",
    cost: None,
    handler: |request, input| logical_ppe_id(request, input.firmware),
}];

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: request.number,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// Hypercall 69: the GameOS partition's logical PPE id in r4, with r5 to
/// r10 cleared; see [`cellgov_ps3_abi::lv1::ppe`].
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the logical PPE id for `firmware`.
pub(crate) fn logical_ppe_id(
    request: &HypercallRequest,
    firmware: Option<&str>,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| logical_ppe_id_handler(fw).is_some()) else {
        return refuse(
            request,
            "a logical PPE id call at a firmware whose lv1 handler is not read",
        );
    };
    let Some(id) = game_os_logical_ppe_id(firmware) else {
        return refuse(
            request,
            "a logical PPE id call at a firmware whose logical PPE id no source gives",
        );
    };
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(
        zero_extended(LV1_SUCCESS),
        [id],
    ))
}

#[cfg(test)]
#[path = "tests/ppe_tests.rs"]
mod tests;
