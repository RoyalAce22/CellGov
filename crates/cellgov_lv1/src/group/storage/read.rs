//! Hypercall 245, `storage_read`, of the notification device: lv1 reports
//! the storage bus's devices to the partition, one device for each read.
//!
//! The facts are in [`cellgov_ps3_abi::lv1::storage_read`]. lv1
//! registers each device before it reports it; see
//! [`super::device_registration`]. It completes the read through the
//! storage write's completion function, which sends the event on the
//! partition's port. When no device is left to report, the read waits.

use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv1::call::STORAGE_READ;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_ps3_abi::lv1::storage::{
    NOTIFY_BUFFER_BYTES, NOTIFY_REQUESTS_FULL_STATUS, STORAGE_DEVICE_ID_MAX,
    STORAGE_NOTIFICATION_DEVICE_ID,
};
use cellgov_ps3_abi::lv1::storage_read::{
    storage_read_handler, NOTIFY_READ_BAD_REQUEST_STATUS, NOTIFY_READ_COMPLETION_STATUS,
    NOTIFY_READ_DEVICE_EVENT, NOTIFY_READ_DEVICE_EVENT_BIT, NOTIFY_READ_FLAGS,
    NOTIFY_READ_QUEUED_EVENT_BIT, NOTIFY_READ_SECTOR_COUNT, NOTIFY_READ_START_SECTOR,
    NOTIFY_READ_TYPE0_EVENT_BIT, STORAGE_READ_NOT_READY_STATUS, STORAGE_READ_NO_DEVICE_STATUS,
};
use cellgov_ps3_abi::lv1::storage_registration::{registration, StorageDevice};

use super::device_registration::register_device;
use super::registration::{NotifyRequest, StorageChange, WaitingRead};
use super::write::{buffer_real, free_tag, lv1_answer, BufferPlace, NotifyState};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::group::interrupt::raise::{self, RaiseState};
use crate::group::memory::region::real_mode_size;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: STORAGE_READ,
    name: "storage_read",
    cost: None,
    handler: |request, input| {
        let interrupt = input.groups.interrupt();
        storage_read(
            request,
            input.firmware,
            &NotifyState {
                requests: input.groups.storage(),
                devices: input.groups.device_bus(),
                regions: input.groups.memory().regions(),
                memory: input.storage,
                raise: RaiseState {
                    bitmaps: interrupt.bitmaps(),
                    ports: interrupt.ports(),
                    paused: interrupt.paused(),
                    storage: input.storage,
                },
            },
        )
    },
}];

/// The call at a firmware whose lv1 handler no image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("a storage_read at a firmware whose lv1 handler is not read");

/// The call for a device from 0 to 6.
///
/// lv1 calls the read method of the device's object
/// [OFW-4.93 lv1 a:0x2515F8-0x25160C]. CellGov models no storage device
/// object.
const DEVICE_OBJECT: UnmodelledError = UnmodelledError::new(
    "a storage_read of a device from 0 to 6, whose device object CellGov does not model",
);

/// The notification read whose buffer CellGov holds no storage for.
///
/// The refusal covers these buffers:
/// - each buffer that the storage write refuses for this reason;
/// - a buffer whose bytes the caller's memory does not hold in full.
const BUFFER: UnmodelledError = UnmodelledError::new(
    "a notification storage_read whose 512-byte buffer CellGov holds no storage for",
);

/// The notification read with the type-0 event bit set in the event
/// mask.
///
/// lv1 reports a type-0 event while the record's index at +56 is below
/// the bus's count at +3784 [OFW-4.93 lv1 a:0x25946C-0x259520]. CellGov
/// models neither word.
const TYPE0_EVENTS: UnmodelledError = UnmodelledError::new(
    "a notification storage_read with event mask bit 0, whose type-0 events CellGov does not model",
);

/// The notification read with the queued event bit set in the event
/// mask, and no device left to report.
///
/// lv1 reports an event from the record's queue at +1392
/// [OFW-4.93 lv1 a:0x2595EC-0x2596E4]. CellGov models no queue.
const QUEUED_EVENTS: UnmodelledError = UnmodelledError::new(
    "a notification storage_read with event mask bit 2, whose queued events CellGov does not model",
);

/// The device report at a firmware whose bus table no source gives.
const UNSOURCED_TABLE: UnmodelledError = UnmodelledError::new(
    "a notification storage_read of device reports at a firmware whose bus table no source gives",
);

/// Hypercall 245: a read of the storage device r3.
///
/// A read of device -1 is a notification read. A call that fails one of
/// lv1's checks gets lv1's own r3, zero-extended, with r4 to r10 cleared,
/// in lv1's order:
/// 1. a device id above 6, other than -1, gets
///    [`STORAGE_READ_NO_DEVICE_STATUS`];
/// 2. a partition that did not open the storage bus gets
///    [`STORAGE_READ_NOT_READY_STATUS`];
/// 3. a start sector, a sector count or flags that lv1 does not accept
///    get [`NOTIFY_READ_BAD_REQUEST_STATUS`];
/// 4. a call while every tag is held gets
///    [`NOTIFY_REQUESTS_FULL_STATUS`].
///
/// A call that passes them takes the lowest free tag, which r4 holds. A
/// buffer that ends past its allocated region then gets lv1's
/// translation status in r3. The request keeps the tag and does not
/// complete. Otherwise r3 is 0, and the event mask selects the answer:
/// - With [`NOTIFY_READ_DEVICE_EVENT_BIT`] set, while a device of the
///   bus table is not reported, the answer reports the next device; see
///   [`report`].
/// - Otherwise the read waits. The answer carries the waiting read, and
///   the request under the tag with no completion.
///
/// The model makes these assumptions:
/// - The partition's open record is in the bus's list, because its open
///   made the record. So lv1's walk of the list passes.
/// - The caller is the game OS partition, whose id is not 1. So lv1 runs
///   the registration before each report.
/// - The storage bus is in
///   [`cellgov_ps3_abi::lv1::storage::STORAGE_BUS_COMPLETING_STATE`], as
///   the storage write assumes. So the completion function completes the
///   report's request in the call, and sends the event
///   [OFW-4.93 lv1 a:0x256464-0x256470].
/// - The partition's open record holds the event mask of the
///   partition's last notification write, or 0 before the first write.
/// - lv1's device-ready event ran for each device of the bus table
///   before the game OS opened the bus. So the count at +3792 is the
///   number of the table's devices. The capture of the registration fits
///   this.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 names a device from 0 to 6;
/// - CellGov holds no storage for the buffer;
/// - the event mask sets [`NOTIFY_READ_TYPE0_EVENT_BIT`];
/// - the event mask sets [`NOTIFY_READ_QUEUED_EVENT_BIT`], and no device
///   is left to report;
/// - no source gives the bus table for `firmware`;
/// - the report is a refusal.
pub(crate) fn storage_read(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &NotifyState<'_>,
) -> HypercallDisposition {
    if firmware.and_then(storage_read_handler).is_none() {
        return UNREAD_FIRMWARE.refusal(request);
    }
    let device = request.r(3);
    if device <= STORAGE_DEVICE_ID_MAX {
        return DEVICE_OBJECT.refusal(request);
    }
    if device != STORAGE_NOTIFICATION_DEVICE_ID {
        return lv1_answer(STORAGE_READ_NO_DEVICE_STATUS);
    }
    if state.devices.storage_bus_open().is_none() {
        return lv1_answer(STORAGE_READ_NOT_READY_STATUS);
    }
    let accepted = request.r(5) == NOTIFY_READ_START_SECTOR
        && request.r(6) == NOTIFY_READ_SECTOR_COUNT
        && request.r(7) == NOTIFY_READ_FLAGS;
    if !accepted {
        return lv1_answer(NOTIFY_READ_BAD_REQUEST_STATUS);
    }
    let Some(tag) = free_tag(state.requests) else {
        return lv1_answer(NOTIFY_REQUESTS_FULL_STATUS);
    };
    let rm_size = firmware.and_then(real_mode_size);
    let real = match buffer_real(state.regions, rm_size, request.r(8)) {
        BufferPlace::Real(real) if state.memory.holds(real, NOTIFY_BUFFER_BYTES) => real,
        BufferPlace::Real(_) | BufferPlace::Unheld => return BUFFER.refusal(request),
        BufferPlace::Lv1(status) => {
            let mut answer = HypercallAnswer::lv1_status(zero_extended(status), [u64::from(tag)]);
            answer.push_change(StorageChange::NotifyRequested(NotifyRequest {
                tag,
                completion: None,
            }));
            return HypercallDisposition::Answered(answer);
        }
    };
    let mask = state.requests.notify_event_mask().unwrap_or(0);
    if mask & NOTIFY_READ_TYPE0_EVENT_BIT != 0 {
        return TYPE0_EVENTS.refusal(request);
    }
    if mask & NOTIFY_READ_DEVICE_EVENT_BIT != 0 {
        let Some(table) = firmware.and_then(registration) else {
            return UNSOURCED_TABLE.refusal(request);
        };
        let next = usize::try_from(state.requests.reported_devices())
            .ok()
            .and_then(|index| table.devices.get(index));
        if let Some(device) = next {
            return report(request, firmware, state, device, real, tag);
        }
    }
    if mask & NOTIFY_READ_QUEUED_EVENT_BIT != 0 {
        return QUEUED_EVENTS.refusal(request);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [u64::from(tag)]);
    answer.push_change(StorageChange::NotifyRequested(NotifyRequest {
        tag,
        completion: None,
    }));
    answer.push_change(StorageChange::NotifyReadWaiting(WaitingRead {
        buffer: real,
        tag,
    }));
    HypercallDisposition::Answered(answer)
}

/// lv1's report of `device` to the partition, for a read under `tag`
/// whose buffer is at the real address `buffer`.
///
/// The answer gives r3 = 0 and r4 = the tag. It carries these effects and
/// changes, in lv1's order:
/// - the registration of the device;
/// - the write of [`NOTIFY_READ_DEVICE_EVENT`] and the device's
///   [`StorageDevice::notify_entry`] at the buffer;
/// - the report, which raises the reported count and ends a waiting read;
/// - the request under the tag, completed with
///   [`NOTIFY_READ_COMPLETION_STATUS`];
/// - lv1's event on the port that the partition connected to the storage
///   bus, when the partition connected one.
///
/// The report is a refusal when the registration or the event is a
/// refusal.
fn report(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &NotifyState<'_>,
    device: &StorageDevice,
    buffer: u64,
    tag: u8,
) -> HypercallDisposition {
    let registered = match register_device(firmware, device.id) {
        Ok(change) => change,
        Err(error) => return error.refusal(request),
    };
    let bytes: Vec<u8> = std::iter::once(NOTIFY_READ_DEVICE_EVENT)
        .chain(device.notify_entry())
        .flat_map(u64::to_be_bytes)
        .collect();
    let Some(range) = ByteRange::new(GuestAddr::new(buffer), bytes.len() as u64) else {
        return BUFFER.refusal(request);
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [u64::from(tag)]);
    answer.push_change(registered);
    answer.effects.push(Effect::SharedWriteIntent {
        range,
        bytes: WritePayload::new(bytes),
        source: request.unit,
        source_time: request.tick,
    });
    answer.push_change(StorageChange::NotifyDeviceReported);
    answer.push_change(StorageChange::NotifyRequested(NotifyRequest {
        tag,
        completion: Some(NOTIFY_READ_COMPLETION_STATUS),
    }));
    if let Some(port) = state.devices.storage_bus_port() {
        if let Err(error) = raise::event(request, &state.raise, port, &mut answer) {
            return error.refusal(request);
        }
    }
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/read_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/read_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/read_event_tests.rs"]
mod event_tests;
