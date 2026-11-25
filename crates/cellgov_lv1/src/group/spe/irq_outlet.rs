//! Hypercall 78, `get_spe_irq_outlet`: the call gives the id of the irq
//! outlet of one interrupt class of a logical SPE.
//!
//! lv1 makes the three outlets of a type 1 SPE in
//! `construct_logical_spe`, and each takes an id from the partition's
//! object table; see [`cellgov_ps3_abi::lv1::spe`]. The partition's other
//! objects decide those ids. So the handler answers the ids that a
//! console capture gives for the SPE's id, and changes no state.

use cellgov_ps3_abi::lv1::call::GET_SPE_IRQ_OUTLET;
use cellgov_ps3_abi::lv1::spe::{game_os_spe_by_id, spe_irq_outlet_handler, SPE_IRQ_CLASSES};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::construct::LogicalSpeState;
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: GET_SPE_IRQ_OUTLET,
    name: "get_spe_irq_outlet",
    cost: None,
    handler: |request, input| irq_outlet(request, input.firmware, input.groups.spe().spes()),
}];

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: request.number,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// Hypercall 78: the outlet id of the class r4 of the SPE r3 in r4, with
/// r5 to r10 cleared.
///
/// lv1 leaves r4 as a stack value when the call fails, and no source
/// gives that value. So the call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r4 is above 2, which lv1 refuses;
/// - r3 names no SPE that the hypervisor holds;
/// - no source gives the outlet ids of the SPE at `firmware`.
pub(crate) fn irq_outlet(
    request: &HypercallRequest,
    firmware: Option<&str>,
    spes: &LogicalSpeState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| spe_irq_outlet_handler(fw).is_some()) else {
        return refuse(
            request,
            "an SPE irq outlet call at a firmware whose lv1 handler is not read",
        );
    };
    let Some(class) = usize::try_from(request.r(4))
        .ok()
        .filter(|&class| class < SPE_IRQ_CLASSES)
    else {
        return refuse(
            request,
            "an SPE irq outlet call for a class above 2, whose r4 lv1 leaves as a stack value",
        );
    };
    let Some(spe) = spes.spe(request.r(3)) else {
        return refuse(
            request,
            "an SPE irq outlet call on an spe id that the hypervisor did not construct",
        );
    };
    let Some(call) = game_os_spe_by_id(firmware, spe.id) else {
        return refuse(
            request,
            "an SPE irq outlet call on an SPE whose outlet ids no source gives",
        );
    };
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(
        zero_extended(LV1_SUCCESS),
        [call.outlets[class]],
    ))
}

#[cfg(test)]
#[path = "tests/irq_outlet_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/irq_outlet_capture_tests.rs"]
mod capture_tests;
