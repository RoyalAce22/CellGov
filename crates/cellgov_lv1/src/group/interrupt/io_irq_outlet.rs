//! Hypercall 120, `construct_io_irq_outlet`: the call gives a device
//! interrupt an irq outlet of the calling partition.
//!
//! lv1 takes the outlet's id from the partition's object table, so the
//! partition's other objects decide it; see
//! [`cellgov_ps3_abi::lv1::io_irq_outlet`]. No source gives the id of a
//! call of the GameOS kernel's boot. So the handler answers only the
//! interrupt id check, which lv1 makes before it constructs anything, and
//! keeps no state.

use cellgov_ps3_abi::lv1::call::CONSTRUCT_IO_IRQ_OUTLET;
use cellgov_ps3_abi::lv1::io_irq_outlet::{
    io_irq_outlet_handler, IO_IRQ_INTERRUPT_ID_MAX, IO_IRQ_OUTLET_ID_R3,
};

use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: CONSTRUCT_IO_IRQ_OUTLET,
    name: "construct_io_irq_outlet",
    cost: None,
    handler: |request, input| construct_outlet(request, input.firmware),
}];

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: request.number,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// Hypercall 120: an irq outlet for the device interrupt r3.
///
/// An interrupt id above [`IO_IRQ_INTERRUPT_ID_MAX`] gets lv1's own r3,
/// [`IO_IRQ_OUTLET_ID_R3`], with r4 to r10 cleared.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - the interrupt id passes the check, because no source gives the id
///   of the outlet that lv1 constructs.
pub(crate) fn construct_outlet(
    request: &HypercallRequest,
    firmware: Option<&str>,
) -> HypercallDisposition {
    if firmware.and_then(io_irq_outlet_handler).is_none() {
        return refuse(
            request,
            "an io irq outlet call at a firmware whose lv1 handler is not read",
        );
    }
    if request.r(3) > IO_IRQ_INTERRUPT_ID_MAX {
        return HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            IO_IRQ_OUTLET_ID_R3,
            [],
        ));
    }
    refuse(
        request,
        "an io irq outlet call whose outlet id no source gives",
    )
}

#[cfg(test)]
#[path = "tests/io_irq_outlet_tests.rs"]
mod tests;
