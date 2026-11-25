//! Hypercall 191, `net_set_interrupt_status_indicator`: lv1 keeps the
//! address where it reports the network device's interrupt status.
//!
//! The handler answers the call for the indicator addresses in the
//! partition's real mode region; see [`cellgov_ps3_abi::lv1::network`].

use cellgov_ps3_abi::lv1::call::NET_SET_INTERRUPT_STATUS_INDICATOR;
use cellgov_ps3_abi::lv1::interrupt::region_lookup_base;
use cellgov_ps3_abi::lv1::network::{
    status_indicator_device_r3, status_indicator_handler, STATUS_INDICATOR_ADDRESS_R3,
    STATUS_INDICATOR_ALIGN, STATUS_INDICATOR_BUS, STATUS_INDICATOR_DEVICE, STATUS_INDICATOR_R6_R3,
    STATUS_INDICATOR_SET_R3, STATUS_INDICATOR_WRONG_STATE_R3,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::state_word::{state_word, StateWord};
use super::{NetworkChange, NetworkState};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::device_bus::{DeviceBusState, OpenDevice};
use crate::group::hypercall::Hypercall;
use crate::group::memory::region::real_mode_size;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: NET_SET_INTERRUPT_STATUS_INDICATOR,
    name: "net_set_interrupt_status_indicator",
    cost: None,
    handler: |request, input| {
        set_status_indicator(
            request,
            input.firmware,
            input.groups.device_bus(),
            input.groups.network(),
        )
    },
}];

/// The network device: device 0 of bus 1.
pub(super) const NETWORK_DEVICE: OpenDevice = OpenDevice {
    bus: STATUS_INDICATOR_BUS,
    device: STATUS_INDICATOR_DEVICE,
};

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError = UnmodelledError::new(
    "a net_set_interrupt_status_indicator at a firmware whose lv1 handler is not read",
);

/// The call before the partition ever opened the network device.
const DEVICE_NOT_OPEN: UnmodelledError = UnmodelledError::new(
    "a net_set_interrupt_status_indicator before the partition opened the network device, whose \
     state word lv1's init sets",
);

/// The call at a firmware whose real mode size no source gives.
const NO_REAL_MODE_SIZE: UnmodelledError = UnmodelledError::new(
    "a net_set_interrupt_status_indicator at a firmware whose real mode size no source gives",
);

/// The call for an indicator outside the real mode region.
const OUTSIDE_REAL_MODE: UnmodelledError = UnmodelledError::new(
    "a network interrupt status indicator outside the partition's real mode region",
);

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 191: sets r5 as the interrupt status indicator of device r4
/// of bus r3.
///
/// The answer clears r4 to r10. A call that fails one of lv1's checks
/// gets lv1's own r3; see [`cellgov_ps3_abi::lv1::network`] for the
/// checks and their order. A call that passes them gets 0, and the
/// answer carries the indicator's address.
///
/// After a close of the network device, the state word is 3, so the call
/// gets [`STATUS_INDICATOR_WRONG_STATE_R3`] and changes no state.
///
/// The model treats the real mode region as a region that passes lv1's
/// class check and its page object check. The kernel closes the network
/// device when this call fails. The network device of the reference
/// console works, so the console's call passes both checks.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - the partition never opened the network device;
/// - no source gives the real mode size for `firmware`;
/// - r5 is outside the real mode region.
pub(crate) fn set_status_indicator(
    request: &HypercallRequest,
    firmware: Option<&str>,
    devices: &DeviceBusState,
    network: &NetworkState,
) -> HypercallDisposition {
    let Some(firmware) = firmware.filter(|fw| status_indicator_handler(fw).is_some()) else {
        return UNREAD_FIRMWARE.refusal(request);
    };
    if let Some(r3) = status_indicator_device_r3(request.r(3), request.r(4)) {
        return lv1_answer(r3);
    }
    match state_word(devices, network) {
        StateWord::Set => {}
        StateWord::TornDown => return lv1_answer(STATUS_INDICATOR_WRONG_STATE_R3),
        StateWord::Unread => return DEVICE_NOT_OPEN.refusal(request),
    }
    if request.r(6) != 0 {
        return lv1_answer(STATUS_INDICATOR_R6_R3);
    }
    let address = request.r(5);
    if !address.is_multiple_of(STATUS_INDICATOR_ALIGN) {
        return lv1_answer(STATUS_INDICATOR_ADDRESS_R3);
    }
    if region_lookup_base(address) != 0 {
        return OUTSIDE_REAL_MODE.refusal(request);
    }
    let Some(rm_size) = real_mode_size(firmware) else {
        return NO_REAL_MODE_SIZE.refusal(request);
    };
    if address >= rm_size {
        return lv1_answer(STATUS_INDICATOR_ADDRESS_R3);
    }
    if network.status_indicator().is_some() {
        return lv1_answer(STATUS_INDICATOR_SET_R3);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(NetworkChange::StatusIndicatorSet(address));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/status_indicator_tests.rs"]
mod tests;
