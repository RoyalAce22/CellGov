//! Hypercall 194, `net_control`: the partition sends a request to the
//! network device.
//!
//! The handler answers these requests; see
//! [`cellgov_ps3_abi::lv1::network`]:
//! - request 1, which gives the device's MAC address. CellGov serves the
//!   MAC address of its synthetic console identity, so no answer holds a
//!   console's address.
//! - request 4, which gives the VLAN id of a selector.

use cellgov_ps3_abi::lv1::call::NET_CONTROL;
use cellgov_ps3_abi::lv1::network::{
    net_control_device_r3, net_control_handler, net_control_request_range_r3,
    net_control_vlan_selector, VlanSelector, NET_CONTROL_GET_MAC_ADDRESS, NET_CONTROL_GET_VLAN_ID,
    NET_CONTROL_VLAN_SELECTOR_R3,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::indicator::NETWORK_DEVICE;
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::device_bus::DeviceBusState;
use crate::group::hypercall::Hypercall;
use crate::identity::{ConsoleIdentity, SYNTHETIC_CONSOLE_IDENTITY};
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: NET_CONTROL,
    name: "net_control",
    cost: None,
    handler: |request, input| {
        net_control(
            request,
            input.firmware,
            input.groups.device_bus(),
            &SYNTHETIC_CONSOLE_IDENTITY,
        )
    },
}];

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("a net_control at a firmware whose lv1 handler is not read");

/// The call before the partition opened the network device.
const DEVICE_NOT_OPEN: UnmodelledError = UnmodelledError::new(
    "a net_control before the partition opened the network device, whose state word lv1's init \
     sets",
);

/// The call for a request code that only the privilege bit passes.
const PRIVILEGED_REQUEST: UnmodelledError = UnmodelledError::new(
    "a net_control request above 11, whose check reads a privilege bit that no lv1 reading gives \
     for the partition",
);

/// The call for a request code from 2 to 11 other than 4.
const UNMODELLED_REQUEST: UnmodelledError = UnmodelledError::new(
    "a net_control request from 2 to 11 other than 4, whose answer CellGov does not model",
);

/// The call for VLAN selector 5.
const FLAGGED_VLAN_SELECTOR: UnmodelledError = UnmodelledError::new(
    "a net_control VLAN id for selector 5, whose answer reads a byte of the network device's \
     global object that no lv1 reading gives",
);

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 194: sends request r5 with the inputs r6 to r8 to device r4
/// of bus r3.
///
/// A call that fails one of lv1's checks gets lv1's own r3 with r4 to
/// r10 cleared; see [`cellgov_ps3_abi::lv1::network`] for the checks and
/// their order. A call that passes them gets these answers:
/// - Request 1 gets 0 in r3, the MAC address of `identity` in r4, and 0
///   in r5 to r10.
/// - Request 4 gets 0 in r3, the VLAN id of the selector in r6 in r4, and
///   0 in r5 to r10. A selector without an id gets
///   [`NET_CONTROL_VLAN_SELECTOR_R3`] with r4 to r10 cleared.
///
/// The model makes these assumptions:
/// - An open network device has the state word 2, because the open sets
///   it.
/// - The device's flag byte at +0 is not 0, as the console capture reads
///   it. No lv1 code read stores that byte.
/// - The doubleword that request 1 reads is the device's MAC address.
///   CellGov serves the address of `identity` there.
/// - Bit 63 of the doubleword at +32 of the global object is 0, so
///   request 4 gives the ids of its table. No lv1 code read stores that
///   doubleword, and a console read of it reads the page that holds the
///   MAC address. When the bit is set, lv1 gives
///   [`NET_CONTROL_VLAN_NONE_R3`](cellgov_ps3_abi::lv1::network::NET_CONTROL_VLAN_NONE_R3)
///   for each selector instead.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - the partition did not open the network device;
/// - r5 is a code that only the privilege bit passes;
/// - r5 is a code from 2 to 11 other than 4;
/// - r5 is 4 and r6 is the selector 5, whose id the byte at +28 of the
///   global object selects.
pub(crate) fn net_control(
    request: &HypercallRequest,
    firmware: Option<&str>,
    devices: &DeviceBusState,
    identity: &ConsoleIdentity,
) -> HypercallDisposition {
    if firmware.and_then(net_control_handler).is_none() {
        return UNREAD_FIRMWARE.refusal(request);
    }
    if let Some(r3) = net_control_device_r3(request.r(3), request.r(4)) {
        return lv1_answer(r3);
    }
    if !devices.is_open(NETWORK_DEVICE) {
        return DEVICE_NOT_OPEN.refusal(request);
    }
    let code = request.r(5);
    if let Some(r3) = net_control_request_range_r3(code, true) {
        return lv1_answer(r3);
    }
    if net_control_request_range_r3(code, false).is_some() {
        return PRIVILEGED_REQUEST.refusal(request);
    }
    match code {
        NET_CONTROL_GET_MAC_ADDRESS => success(identity.mac_address),
        NET_CONTROL_GET_VLAN_ID => vlan_id(request),
        _ => UNMODELLED_REQUEST.refusal(request),
    }
}

/// r3 = 0 with `r4` in r4 and r5 to r10 cleared.
fn success(r4: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(
        zero_extended(LV1_SUCCESS),
        [r4, 0],
    ))
}

/// Request 4 for the selector in r6, with bit 63 of the capability
/// doubleword taken as 0.
fn vlan_id(request: &HypercallRequest) -> HypercallDisposition {
    match net_control_vlan_selector(request.r(6)) {
        VlanSelector::Fixed(id) => success(id),
        VlanSelector::Flagged => FLAGGED_VLAN_SELECTOR.refusal(request),
        VlanSelector::Illegal => lv1_answer(NET_CONTROL_VLAN_SELECTOR_R3),
    }
}

#[cfg(test)]
#[path = "tests/control_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/control_vlan_tests.rs"]
mod vlan_tests;
