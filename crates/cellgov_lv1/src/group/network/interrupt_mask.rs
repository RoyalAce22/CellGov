//! Hypercall 193, `net_set_interrupt_mask`: lv1 keeps the network
//! device's interrupt mask.
//!
//! The handler answers the call for the network device; see
//! [`cellgov_ps3_abi::lv1::network`]. lv1 keeps the mask that the last
//! call set, and the state holds it.

use cellgov_ps3_abi::lv1::call::NET_SET_INTERRUPT_MASK;
use cellgov_ps3_abi::lv1::network::{
    interrupt_mask_device_r3, interrupt_mask_handler, INTERRUPT_MASK_R6_R3,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::indicator::NETWORK_DEVICE;
use super::NetworkChange;
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::device_bus::DeviceBusState;
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: NET_SET_INTERRUPT_MASK,
    name: "net_set_interrupt_mask",
    cost: None,
    handler: |request, input| {
        set_interrupt_mask(request, input.firmware, input.groups.device_bus())
    },
}];

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("a net_set_interrupt_mask at a firmware whose lv1 handler is not read");

/// The call before the partition opened the network device.
const DEVICE_NOT_OPEN: UnmodelledError = UnmodelledError::new(
    "a net_set_interrupt_mask before the partition opened the network device, whose state word \
     lv1's init sets",
);

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 193: sets r5 as the interrupt mask of device r4 of bus r3.
///
/// The answer clears r4 to r10. A call that fails one of lv1's checks
/// gets lv1's own r3; see [`cellgov_ps3_abi::lv1::network`] for the
/// checks and their order. A call that passes them gets 0, and the
/// answer carries the mask. lv1 checks no bit of the mask.
///
/// The model assumes that an open network device has the state word 2,
/// because the open sets it.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - the partition did not open the network device.
pub(crate) fn set_interrupt_mask(
    request: &HypercallRequest,
    firmware: Option<&str>,
    devices: &DeviceBusState,
) -> HypercallDisposition {
    if firmware.and_then(interrupt_mask_handler).is_none() {
        return UNREAD_FIRMWARE.refusal(request);
    }
    if let Some(r3) = interrupt_mask_device_r3(request.r(3), request.r(4)) {
        return lv1_answer(r3);
    }
    if !devices.is_open(NETWORK_DEVICE) {
        return DEVICE_NOT_OPEN.refusal(request);
    }
    if request.r(6) != 0 {
        return lv1_answer(INTERRUPT_MASK_R6_R3);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(NetworkChange::InterruptMaskSet(request.r(5)));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/interrupt_mask_tests.rs"]
mod tests;
