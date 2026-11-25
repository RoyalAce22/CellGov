//! Hypercall 246, `storage_write`, to the notification device: lv1 takes
//! a request for the device events of the storage bus.
//!
//! The facts are in [`cellgov_ps3_abi::lv1::storage`]. lv1 completes the
//! request in the call. It then sends an event on the port that the
//! partition connected to the storage bus. The event itself is the
//! interrupt group's [`raise::event`].

use cellgov_mem::{be::read_u64, ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::call::STORAGE_WRITE;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_ps3_abi::lv1::storage::{
    storage_write_handler, NOTIFY_BUFFER_BYTES, NOTIFY_COMPLETION_STATUS, NOTIFY_EVENT_MASK_OFFSET,
    NOTIFY_REQUESTS_FULL_STATUS, NOTIFY_REQUEST_TAGS, NOTIFY_WRITE_BAD_REQUEST_STATUS,
    NOTIFY_WRITE_BUFFER_PAST_REGION_STATUS, NOTIFY_WRITE_COMMAND, NOTIFY_WRITE_FLAGS,
    NOTIFY_WRITE_SECTOR_COUNT, NOTIFY_WRITE_START_SECTOR, OTHER_PARTITION_NOTIFY_TAGS,
    STORAGE_DEVICE_ID_MAX, STORAGE_NOTIFICATION_DEVICE_ID, STORAGE_WRITE_NOT_READY_STATUS,
    STORAGE_WRITE_NO_DEVICE_STATUS,
};

use super::registration::{NotifyRequest, StorageChange, StorageState};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::device_bus::DeviceBusState;
use crate::group::hypercall::Hypercall;
use crate::group::interrupt::raise::{self, RaiseState};
use crate::group::memory::region::{real_mode_size, MemoryState};
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: STORAGE_WRITE,
    name: "storage_write",
    cost: None,
    handler: |request, input| {
        let interrupt = input.groups.interrupt();
        storage_write(
            request,
            input.firmware,
            &WriteState {
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

/// The state that the hypervisor holds for a storage write.
pub(crate) struct WriteState<'a> {
    /// The notification requests that lv1 holds for the partition.
    pub requests: &'a StorageState,
    /// The storage bus open and its connected port.
    pub devices: &'a DeviceBusState,
    /// The partition's memory regions, which locate the buffer.
    pub regions: &'a MemoryState,
    /// The caller's real address space, which holds the request's buffer.
    pub memory: &'a GuestMemory,
    /// The interrupt state that the event reads.
    pub raise: RaiseState<'a>,
}

/// The call at a firmware whose lv1 handler no image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("a storage_write at a firmware whose lv1 handler is not read");

/// The call for a device from 0 to 6.
///
/// lv1 calls the write method of the device's object
/// [OFW-4.93 lv1 a:0x2513A4-0x251484], and CellGov models no storage
/// device object.
const DEVICE_OBJECT: UnmodelledError = UnmodelledError::new(
    "a storage_write to a device from 0 to 6, whose device object CellGov does not model",
);

/// The notification write whose buffer CellGov holds no storage for.
///
/// lv1's translation of the buffer [OFW-4.93 lv1 a:0x254484-0x2545A0]
/// refuses some of these buffers with statuses that no read of the
/// translation's region lookup settles:
/// - a real mode buffer that ends past the real mode region;
/// - a buffer in a region that gets no storage;
/// - a buffer above the real mode region and outside every allocated
///   region;
/// - a real mode buffer at a firmware whose real mode size no source
///   gives.
const BUFFER: UnmodelledError = UnmodelledError::new(
    "a notification storage_write whose 512-byte buffer CellGov holds no storage for",
);

/// lv1's status `status`, zero-extended, with r4 to r10 cleared.
fn lv1_answer(status: i64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(zero_extended(status), []))
}

/// Where CellGov finds the bytes of a notification buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BufferPlace {
    /// The real address of CellGov's storage of the bytes.
    Real(u64),
    /// lv1's translation refuses the bytes with this status.
    Lv1(i64),
    /// CellGov holds no storage for the bytes.
    Unheld,
}

/// Where CellGov finds the [`NOTIFY_BUFFER_BYTES`] bytes at the logical
/// partition address `address`.
///
/// lv1's translation finds the partition's memory region that holds the
/// address [OFW-4.93 lv1 a:0x2544D4-0x2544E8]. It checks that the bytes
/// end in the region [OFW-4.93 lv1 a:0x254520-0x254538], and adds the
/// offset to the page's address [OFW-4.93 lv1 a:0x25453C-0x25457C].
/// CellGov keeps the pages of an allocated region in one range of
/// storage. An address outside every allocated region is a real mode
/// address, which CellGov keeps at the same real address. The real mode
/// region holds the `real_mode_size` bytes from address 0, so a buffer
/// outside every allocated region that does not end in it is unheld.
/// A `real_mode_size` of `None` makes each such buffer unheld. Bytes
/// that end past their allocated region get
/// [`NOTIFY_WRITE_BUFFER_PAST_REGION_STATUS`].
fn buffer_real(regions: &MemoryState, real_mode_size: Option<u64>, address: u64) -> BufferPlace {
    let Some(end) = address.checked_add(NOTIFY_BUFFER_BYTES) else {
        return BufferPlace::Unheld;
    };
    let found = (0u64..)
        .zip(regions.regions())
        .find(|(_, r)| address >= r.lpar_address && address - r.lpar_address < r.size);
    let Some((index, region)) = found else {
        // The storage above the real mode region holds pages that lv1
        // gives the partition at no logical partition address of their
        // own (see `crate::group::memory::storage`). lv1's translation
        // refuses an address outside the partition's memory
        // [OFW-4.93 lv1 a:0x254484-0x2545A0].
        return match real_mode_size {
            Some(size) if end <= size => BufferPlace::Real(address),
            _ => BufferPlace::Unheld,
        };
    };
    if end - region.lpar_address > region.size {
        return BufferPlace::Lv1(NOTIFY_WRITE_BUFFER_PAST_REGION_STATUS);
    }
    regions
        .storage_real(index)
        .and_then(|real| real.checked_add(address - region.lpar_address))
        .map_or(BufferPlace::Unheld, BufferPlace::Real)
}

/// The doublewords at +0 and at [`NOTIFY_EVENT_MASK_OFFSET`] of the
/// buffer at the real address `address`, or `None` when `memory` does not
/// hold all [`NOTIFY_BUFFER_BYTES`] bytes of it.
fn read_buffer(memory: &GuestMemory, address: u64) -> Option<(u64, u64)> {
    if !memory.holds(address, NOTIFY_BUFFER_BYTES) {
        return None;
    }
    let head = ByteRange::new(GuestAddr::new(address), NOTIFY_EVENT_MASK_OFFSET + 8)?;
    let bytes = memory.read(head)?;
    Some((read_u64(&bytes, 0), read_u64(&bytes, 8)))
}

/// Hypercall 246: a write to the storage device r3.
///
/// A write to device -1 is a notification write. The answer clears r4
/// to r10, except the tag. A call that fails one of lv1's checks gets
/// lv1's own r3, zero-extended, in lv1's order:
/// 1. a device id above 6, other than -1, gets
///    [`STORAGE_WRITE_NO_DEVICE_STATUS`];
/// 2. a partition that did not open the storage bus gets
///    [`STORAGE_WRITE_NOT_READY_STATUS`];
/// 3. a start sector, a sector count or flags that lv1 does not accept
///    get [`NOTIFY_WRITE_BAD_REQUEST_STATUS`];
/// 4. a buffer that ends past its allocated region gets
///    [`NOTIFY_WRITE_BUFFER_PAST_REGION_STATUS`];
/// 5. a buffer whose doubleword at +0 is not [`NOTIFY_WRITE_COMMAND`]
///    gets [`NOTIFY_WRITE_BAD_REQUEST_STATUS`];
/// 6. a call while every tag is held gets
///    [`NOTIFY_REQUESTS_FULL_STATUS`], and the answer carries the event
///    mask.
///
/// A call that passes them gets 0 and the lowest free tag in r4. The
/// answer carries these changes:
/// - the event mask, the doubleword at +8 of the buffer;
/// - the request under the tag, completed with
///   [`NOTIFY_COMPLETION_STATUS`];
/// - lv1's event on the port that the partition connected to the storage
///   bus, when the partition connected one.
///
/// The model makes these assumptions:
/// - The storage bus is in the state that completes a request in the
///   call, and gives a new request no memory object, as the capture
///   shows after boot.
/// - Partition 1 holds the tags [`OTHER_PARTITION_NOTIFY_TAGS`].
/// - The partition's open record is in the bus's list, because its open
///   made the record. So lv1's second ready check and its walk of the
///   list pass.
/// - The connected port holds a reference, because
///   `construct_event_receive_port` takes one and no answered call drops
///   it.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - r3 names a device from 0 to 6;
/// - CellGov holds no storage for the buffer;
/// - the event is a refusal of [`raise::event`].
pub(crate) fn storage_write(
    request: &HypercallRequest,
    firmware: Option<&str>,
    state: &WriteState<'_>,
) -> HypercallDisposition {
    if firmware.and_then(storage_write_handler).is_none() {
        return UNREAD_FIRMWARE.refusal(request);
    }
    let device = request.r(3);
    if device <= STORAGE_DEVICE_ID_MAX {
        return DEVICE_OBJECT.refusal(request);
    }
    if device != STORAGE_NOTIFICATION_DEVICE_ID {
        return lv1_answer(STORAGE_WRITE_NO_DEVICE_STATUS);
    }
    if state.devices.storage_bus_open().is_none() {
        return lv1_answer(STORAGE_WRITE_NOT_READY_STATUS);
    }
    let accepted = request.r(5) == NOTIFY_WRITE_START_SECTOR
        && request.r(6) == NOTIFY_WRITE_SECTOR_COUNT
        && request.r(7) == NOTIFY_WRITE_FLAGS;
    if !accepted {
        return lv1_answer(NOTIFY_WRITE_BAD_REQUEST_STATUS);
    }
    let rm_size = firmware.and_then(real_mode_size);
    let real = match buffer_real(state.regions, rm_size, request.r(8)) {
        BufferPlace::Real(real) => real,
        BufferPlace::Lv1(status) => return lv1_answer(status),
        BufferPlace::Unheld => return BUFFER.refusal(request),
    };
    let Some((command, mask)) = read_buffer(state.memory, real) else {
        return BUFFER.refusal(request);
    };
    if command != NOTIFY_WRITE_COMMAND {
        return lv1_answer(NOTIFY_WRITE_BAD_REQUEST_STATUS);
    }
    let held = OTHER_PARTITION_NOTIFY_TAGS | state.requests.held_tags();
    let Some(tag) = (0..NOTIFY_REQUEST_TAGS).find(|&tag| held & (1 << tag) == 0) else {
        let mut answer =
            HypercallAnswer::lv1_status(zero_extended(NOTIFY_REQUESTS_FULL_STATUS), []);
        answer.push_change(StorageChange::NotifyEventMaskSet(mask));
        return HypercallDisposition::Answered(answer);
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [u64::from(tag)]);
    answer.push_change(StorageChange::NotifyEventMaskSet(mask));
    answer.push_change(StorageChange::NotifyRequested(NotifyRequest {
        tag,
        completion: Some(NOTIFY_COMPLETION_STATUS),
    }));
    if let Some(port) = state.devices.storage_bus_port() {
        if let Err(error) = raise::event(request, &state.raise, port, &mut answer) {
            return error.refusal(request);
        }
    }
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/write_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/write_console_tests.rs"]
mod console_tests;
