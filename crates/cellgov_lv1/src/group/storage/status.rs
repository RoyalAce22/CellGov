//! Hypercall 249, `storage_get_async_status`, of the notification device:
//! lv1 gives the partition a completed notification request and frees
//! its tag.
//!
//! The facts are in [`cellgov_ps3_abi::lv1::storage_status`].

use cellgov_ps3_abi::lv1::call::STORAGE_GET_ASYNC_STATUS;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_ps3_abi::lv1::storage::{STORAGE_DEVICE_ID_MAX, STORAGE_NOTIFICATION_DEVICE_ID};
use cellgov_ps3_abi::lv1::storage_status::{
    storage_status_handler, ASYNC_STATUS_NONE_STATUS, ASYNC_STATUS_NOT_READY_STATUS,
    ASYNC_STATUS_NO_DEVICE_STATUS, ASYNC_STATUS_SLOT_WORD_FLAG,
};

use super::registration::{StorageChange, StorageState};
use super::write::lv1_answer;
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::device_bus::DeviceBusState;
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: STORAGE_GET_ASYNC_STATUS,
    name: "storage_get_async_status",
    cost: None,
    handler: |request, input| {
        storage_get_async_status(
            request,
            input.firmware,
            input.groups.storage(),
            input.groups.device_bus(),
        )
    },
}];

/// The call at a firmware whose lv1 handler no image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("a storage_get_async_status at a firmware whose lv1 handler is not read");

/// The call for a device from 0 to 6.
///
/// lv1 calls the status method of the device's object
/// [OFW-4.93 lv1 a:0x251EE0-0x251F94]. CellGov models no storage
/// device object.
const DEVICE_OBJECT: UnmodelledError = UnmodelledError::new(
    "a storage_get_async_status for a device from 0 to 6, whose device object CellGov does not model",
);

/// The call with no completed request, for an open whose r5 makes lv1
/// read the slot word.
///
/// lv1 takes the doubleword at +56 of the partition's open slot
/// [OFW-4.93 lv1 a:0x25606C-0x25609C]. CellGov does not model the
/// doubleword.
const SLOT_WORD: UnmodelledError = UnmodelledError::new(
    "a storage_get_async_status of the notification device that reads the open slot's word",
);

/// Hypercall 249: the status of a completed request of the storage
/// device r3.
///
/// For device -1, the answer gives the completed request of the lowest
/// tag. It sets r3 = 0, r4 = the tag and r5 = the request's status, and
/// clears r6 to r10. The answer carries the change that removes the
/// request and frees its tag. Other calls get lv1's own r3,
/// zero-extended, with r4 to r10 cleared, in lv1's order:
/// 1. a device id above 6, other than -1, gets
///    [`ASYNC_STATUS_NO_DEVICE_STATUS`];
/// 2. a partition that did not open the storage bus gets
///    [`ASYNC_STATUS_NOT_READY_STATUS`];
/// 3. a partition with no completed request gets
///    [`ASYNC_STATUS_NONE_STATUS`].
///
/// The model makes these assumptions:
/// - Each request that the model holds is the game OS partition's. The
///   notification write gives it the kind 0, no memory object and 0 at
///   +32.
/// - The game OS partition's open slot holds the r5 of its open.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 names a device from 0 to 6;
/// - the partition has no completed request, and the r5 of its open sets
///   [`ASYNC_STATUS_SLOT_WORD_FLAG`].
pub(crate) fn storage_get_async_status(
    request: &HypercallRequest,
    firmware: Option<&str>,
    requests: &StorageState,
    devices: &DeviceBusState,
) -> HypercallDisposition {
    if firmware.and_then(storage_status_handler).is_none() {
        return UNREAD_FIRMWARE.refusal(request);
    }
    let device = request.r(3);
    if device <= STORAGE_DEVICE_ID_MAX {
        return DEVICE_OBJECT.refusal(request);
    }
    if device != STORAGE_NOTIFICATION_DEVICE_ID {
        return lv1_answer(ASYNC_STATUS_NO_DEVICE_STATUS);
    }
    let Some(open) = devices.storage_bus_open() else {
        return lv1_answer(ASYNC_STATUS_NOT_READY_STATUS);
    };
    let completed = requests
        .notify_requests()
        .find_map(|r| r.completion.map(|status| (r.tag, status)));
    if let Some((tag, status)) = completed {
        let outputs = [u64::from(tag), status];
        let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), outputs);
        answer.push_change(StorageChange::NotifyRequestTaken(tag));
        return HypercallDisposition::Answered(answer);
    }
    if open.r5 & ASYNC_STATUS_SLOT_WORD_FLAG != 0 {
        return SLOT_WORD.refusal(request);
    }
    lv1_answer(ASYNC_STATUS_NONE_STATUS)
}

#[cfg(test)]
#[path = "tests/status_tests.rs"]
mod tests;
