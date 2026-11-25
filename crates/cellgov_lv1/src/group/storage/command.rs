//! Hypercall 248, `storage_send_device_command`: lv1 starts a command on
//! a storage device, and completes it when the device's controller
//! replies.
//!
//! The facts are in [`cellgov_ps3_abi::lv1::storage_command`]. lv1
//! answers the call before the controller replies. The reply's status
//! and data come from the controller, and no read of lv1 gives them. So
//! the request waits, and the model sends no completion event.

use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::call::STORAGE_SEND_DEVICE_COMMAND;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_ps3_abi::lv1::storage::{STORAGE_DEVICE_ID_MAX, STORAGE_NOTIFICATION_DEVICE_ID};
use cellgov_ps3_abi::lv1::storage_command::{
    command_accepted, storage_command_handler, CONTROLLER_QUERY_COMMAND,
    CONTROLLER_QUERY_DATA_BYTES, SEND_COMMAND_BAD_REQUEST_STATUS, SEND_COMMAND_NOT_READY_STATUS,
    SEND_COMMAND_NO_DEVICE_STATUS, SEND_COMMAND_TAGS, SEND_COMMAND_TAGS_FULL_STATUS,
};
use cellgov_ps3_abi::lv1::storage_registration::registration;

use super::registration::{DeviceCommand, StorageChange, StorageState};
use super::write::{buffer_real, lv1_answer, BufferPlace};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::device_bus::DeviceBusState;
use crate::group::hypercall::Hypercall;
use crate::group::memory::region::{real_mode_size, MemoryState};
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: STORAGE_SEND_DEVICE_COMMAND,
    name: "storage_send_device_command",
    cost: None,
    handler: |request, input| {
        storage_send_device_command(
            request,
            input.firmware,
            &CommandState {
                commands: input.groups.storage(),
                devices: input.groups.device_bus(),
                regions: input.groups.memory().regions(),
                memory: input.storage,
            },
        )
    },
}];

/// The state that the hypervisor holds for a device command.
pub(crate) struct CommandState<'a> {
    /// The device commands that lv1 holds for the partition.
    pub commands: &'a StorageState,
    /// The storage bus open and the storage device opens.
    pub devices: &'a DeviceBusState,
    /// The partition's memory regions, which locate the buffers.
    pub regions: &'a MemoryState,
    /// The caller's real address space, which holds the data buffer.
    pub memory: &'a GuestMemory,
}

/// The call at a firmware whose lv1 handler no image read shows.
const UNREAD_FIRMWARE: UnmodelledError = UnmodelledError::new(
    "a storage_send_device_command at a firmware whose lv1 handler is not read",
);

/// The call at a firmware whose storage bus table no source gives.
const UNSOURCED_TABLE: UnmodelledError = UnmodelledError::new(
    "a storage_send_device_command at a firmware whose storage bus table no source gives",
);

/// The call for device -1, from a partition that opened the storage
/// bus.
///
/// The core calls slot 34 of the storage bus object's table
/// [OFW-4.93 lv1 a:0x251D10-0x251D2C]. No read covers that method.
const NOTIFICATION_DEVICE: UnmodelledError = UnmodelledError::new(
    "a storage_send_device_command to the notification device, whose method is not read",
);

/// The call for an open device whose command method no read covers.
const DEVICE_METHOD: UnmodelledError = UnmodelledError::new(
    "a storage_send_device_command to a device whose command method is not read",
);

/// The call whose command buffer CellGov holds no storage for.
const COMMAND_BUFFER: UnmodelledError = UnmodelledError::new(
    "a storage_send_device_command whose command buffer CellGov holds no storage for",
);

/// The call whose data buffer CellGov holds no storage for.
const DATA_BUFFER: UnmodelledError = UnmodelledError::new(
    "a storage_send_device_command whose data buffer CellGov holds no storage for",
);

/// The call whose data buffer ends past the memory region that holds its
/// first byte.
///
/// For a data length other than 0 in the full r8, the request function
/// checks the data buffer before it gives the tag
/// [OFW-4.93 lv1 a:0x2539D0-0x2539D4]. When the list of the device that
/// [OFW-4.93 lv1 a:0x2284C8] walks does not hold the address, the
/// function returns -6 for a region with too few bytes
/// [OFW-4.93 lv1 a:0x253B34-0x253B48] [OFW-4.93 lv1 a:0x253BC0].
/// Otherwise the dispatcher takes the buffer. CellGov models no such
/// list.
const DATA_PAST_REGION: UnmodelledError = UnmodelledError::new(
    "a storage_send_device_command whose data buffer ends past its memory region",
);

/// The call with an accepted command id other than the controller
/// query.
///
/// The dispatcher asks lv1's policy, and starts the command through its
/// own case of the table at 0x2721B8
/// [OFW-4.93 lv1 a:0x272198-0x2721B4]. No read covers those cases.
const OTHER_COMMAND: UnmodelledError =
    UnmodelledError::new("a storage_send_device_command of a command id whose case is not read");

/// The call while a command of the device waits for the controller.
///
/// The dispatcher queues the command in its ring of command slots
/// [OFW-4.93 lv1 a:0x272D48-0x272DCC]. CellGov models no ring.
const QUEUED: UnmodelledError = UnmodelledError::new(
    "a storage_send_device_command while a command of the device waits for its controller",
);

/// The controller query whose data buffer is short or untranslated.
///
/// lv1 completes the request in the call
/// [OFW-4.93 lv1 a:0x27227C-0x2722A8]. No read covers the device's
/// completion state.
const SHORT_DATA: UnmodelledError =
    UnmodelledError::new("a controller query whose data buffer is short or untranslated");

/// The low word of `value`, which lv1 reads with a 32-bit compare.
fn low_word(value: u64) -> u32 {
    (value & u64::from(u32::MAX)) as u32
}

/// lv1's `status`, zero-extended, for a request that the request
/// function gave `tag`: r4 holds the tag, and r5 to r10 are 0.
///
/// The request function stores the tag in the return slot of r4
/// [OFW-4.93 lv1 a:0x253A04]. The command method gives the dispatcher
/// the slot's value [OFW-4.93 lv1 a:0x273038]. On the command method's
/// failure the request method frees the tag and does not write the slot
/// [OFW-4.93 lv1 a:0x256E5C-0x256EF0], and the vector loads r4 from the
/// slot [OFW-4.93 lv1 a:0x1044].
fn tagged_answer(status: i64, tag: u8) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(
        zero_extended(status),
        [u64::from(tag)],
    ))
}

/// Hypercall 248: the command r4 to the storage device r3.
///
/// A call that fails one of lv1's checks gets lv1's own r3,
/// zero-extended, with r4 to r10 cleared except where step 4 says, in
/// lv1's order:
/// 1. a device id above 6 other than -1, or a device that the bus table
///    does not hold, gets [`SEND_COMMAND_NO_DEVICE_STATUS`];
/// 2. a device that the partition did not open gets
///    [`SEND_COMMAND_NOT_READY_STATUS`];
/// 3. a call while each tag of the device is held gets
///    [`SEND_COMMAND_TAGS_FULL_STATUS`]. A data buffer that the request
///    function's check could refuse is then a refusal;
/// 4. a command id that lv1 does not accept, or a command buffer that
///    the translation refuses, gets [`SEND_COMMAND_BAD_REQUEST_STATUS`],
///    with the lowest free tag of the device in r4.
///
/// A controller query that passes them gets 0 and the lowest free tag of
/// the device in r4. The answer carries these effects and changes:
/// - the write of 0 to the three words at +0, +4 and +8 of the data
///   buffer;
/// - the command, which waits for the controller's reply.
///
/// The model makes these assumptions:
/// - The device's object holds no request that CellGov does not hold.
///   The game OS partition is the only partition that opened device 2.
/// - lv1's policy lets the game OS partition send the controller query.
/// - The device's ring and its controller are idle while no command of
///   the device waits. So the dispatcher starts the command in the call.
/// - The controller takes the query.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - no source gives the bus table for `firmware`;
/// - r3 is -1, and the partition opened the storage bus;
/// - r3 names an open device whose command method no read covers;
/// - CellGov holds no storage for a buffer;
/// - the data buffer ends past the memory region that holds its first
///   byte;
/// - the command id is not the controller query;
/// - a command of the device waits;
/// - the data buffer is short or untranslated.
pub(crate) fn storage_send_device_command(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &CommandState<'_>,
) -> HypercallDisposition {
    let Some(handler) = firmware.and_then(storage_command_handler) else {
        return UNREAD_FIRMWARE.refusal(request);
    };
    let device = request.r(3);
    if device == STORAGE_NOTIFICATION_DEVICE_ID {
        if state.devices.storage_bus_open().is_none() {
            return lv1_answer(SEND_COMMAND_NOT_READY_STATUS);
        }
        return NOTIFICATION_DEVICE.refusal(request);
    }
    let Some(table) = firmware.and_then(registration) else {
        return UNSOURCED_TABLE.refusal(request);
    };
    if device > STORAGE_DEVICE_ID_MAX || table.device(device).is_none() {
        return lv1_answer(SEND_COMMAND_NO_DEVICE_STATUS);
    }
    if state.devices.storage_device_open(device).is_none() {
        return lv1_answer(SEND_COMMAND_NOT_READY_STATUS);
    }
    if device != handler.read_device {
        return DEVICE_METHOD.refusal(request);
    }
    let held = state.commands.device_held_tags(device);
    let Some(tag) = (0..SEND_COMMAND_TAGS).find(|&tag| held & (1 << tag) == 0) else {
        return lv1_answer(SEND_COMMAND_TAGS_FULL_STATUS);
    };
    let rm_size = firmware.and_then(real_mode_size);
    let checked_data = match request.r(8) {
        0 => None,
        full => match buffer_real(state.regions, rm_size, request.r(7), full) {
            BufferPlace::Real(real) => Some(real),
            BufferPlace::Lv1(_) => return DATA_PAST_REGION.refusal(request),
            BufferPlace::Unheld => return DATA_BUFFER.refusal(request),
        },
    };
    let command = low_word(request.r(4));
    if !command_accepted(command) {
        return tagged_answer(SEND_COMMAND_BAD_REQUEST_STATUS, tag);
    }
    let command_bytes = u64::from(low_word(request.r(6)));
    if command_bytes != 0 {
        match buffer_real(state.regions, rm_size, request.r(5), command_bytes) {
            BufferPlace::Real(_) => {}
            BufferPlace::Lv1(_) => return tagged_answer(SEND_COMMAND_BAD_REQUEST_STATUS, tag),
            BufferPlace::Unheld => return COMMAND_BUFFER.refusal(request),
        }
    }
    // The dispatcher translates the low word of the data length. The
    // request function found the full length in the buffer's region, so
    // the shorter translation finds it too.
    let data_bytes = u64::from(low_word(request.r(8)));
    let data = checked_data.filter(|_| data_bytes != 0);
    if command != CONTROLLER_QUERY_COMMAND {
        return OTHER_COMMAND.refusal(request);
    }
    if held != 0 {
        return QUEUED.refusal(request);
    }
    let Some(data) = data.filter(|_| data_bytes >= CONTROLLER_QUERY_DATA_BYTES) else {
        return SHORT_DATA.refusal(request);
    };
    controller_query(request, state.memory, device, tag, data, data_bytes)
}

/// lv1's start of the controller query under `tag` on `device`, whose
/// data buffer of `data_bytes` bytes is at the real address `data`.
///
/// The query is a refusal when `memory` does not hold the three words.
fn controller_query(
    request: &HypercallRequest,
    memory: &GuestMemory,
    device: u64,
    tag: u8,
    data: u64,
    data_bytes: u64,
) -> HypercallDisposition {
    let words = ByteRange::new(GuestAddr::new(data), CONTROLLER_QUERY_DATA_BYTES)
        .filter(|_| memory.holds(data, CONTROLLER_QUERY_DATA_BYTES));
    let Some(range) = words else {
        return DATA_BUFFER.refusal(request);
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [u64::from(tag)]);
    answer.effects.push(Effect::SharedWriteIntent {
        range,
        bytes: WritePayload::new(vec![0; 12]),
        source: request.unit,
        source_time: request.tick,
    });
    answer.push_change(StorageChange::DeviceCommandStarted(DeviceCommand {
        device,
        tag,
        command: CONTROLLER_QUERY_COMMAND,
        data,
        data_bytes,
    }));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/command_tests.rs"]
mod tests;
