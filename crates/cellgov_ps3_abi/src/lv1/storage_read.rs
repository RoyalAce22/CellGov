//! lv1's `storage_read` (hypercall 245): the statuses of the reads it
//! refuses, and its notification read of device -1.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`, and from the console capture
//! [`super::storage_registration::REGISTRATION_CAPTURE`]. The repository
//! holds no byte of lv1, only these facts. The path of a read:
//! 1. The handler, at [OFW-4.93 lv1 a:0x308B88] in the live table,
//!    zeroes the return slot of r4, HSPRG0 - 0x6A38
//!    [OFW-4.93 lv1 a:0x308C58]. It calls the read core with the caller's
//!    partition, r3 to r8 and the slot's address
//!    [OFW-4.93 lv1 a:0x308C48-0x308C70]. It returns the core's r3
//!    zero-extended [OFW-4.93 lv1 a:0x308CD8].
//! 2. The core [OFW-4.93 lv1 a:0x2514C8] returns
//!    [`STORAGE_READ_NO_DEVICE_STATUS`] in these cases
//!    [OFW-4.93 lv1 a:0x2514D8-0x25153C] [OFW-4.93 lv1 a:0x2515BC-0x2515C0]:
//!    - a device id above 6, other than -1;
//!    - a device whose object pointer is null. The pointers are a table
//!      of doublewords at +3816 of the storage bus object, indexed by the
//!      device id [OFW-4.93 lv1 a:0x2514E0] [OFW-4.93 lv1 a:0x251510]
//!      [OFW-4.93 lv1 a:0x25152C].
//! 3. For device -1 the core uses the storage bus object as the device
//!    [OFW-4.93 lv1 a:0x251648-0x25164C].
//! 4. The core asks the device's ready method, slot 6 of its virtual
//!    table. A false answer gives [`STORAGE_READ_NOT_READY_STATUS`]
//!    [OFW-4.93 lv1 a:0x251540-0x251584]. For the bus object, the method
//!    is the one that the storage write asks; see [`super::storage`].
//! 5. Otherwise the core calls slot 32 of the virtual table and returns
//!    that method's r3 [OFW-4.93 lv1 a:0x2515F8-0x25161C]. For the bus
//!    object, slot 32 is the notification read method at
//!    [OFW-4.93 lv1 a:0x2592E4] [OFW-4.93 lv1 a:0x35DE30-0x35DE34]
//!    [OFW-4.93 lv1 a:0x3355B0-0x3355B4].
//!
//! # The notification read
//!
//! The method holds the lock at +3576 of the bus object
//! [OFW-4.93 lv1 a:0x259324-0x259348]. It does these steps, and the first
//! failure is the answer:
//! 1. It refuses a start sector, a sector count or flags other than
//!    [`NOTIFY_READ_START_SECTOR`], [`NOTIFY_READ_SECTOR_COUNT`] and
//!    [`NOTIFY_READ_FLAGS`] with [`NOTIFY_READ_BAD_REQUEST_STATUS`]. It
//!    does not read the region id.
//! 2. It walks the open records from +3552 for the caller's partition, at
//!    +40 of each record. It returns [`NOTIFY_READ_NO_RECORD_STATUS`]
//!    when no record holds the partition
//!    [OFW-4.93 lv1 a:0x259364-0x259394].
//! 3. Under the lock at +2672, it gives the request a tag through the
//!    storage write's request function
//!    [OFW-4.93 lv1 a:0x2593E4-0x259440]. That function stores the tag in
//!    the return slot of r4, or returns the storage write's busy status.
//! 4. It translates the [`super::storage::NOTIFY_BUFFER_BYTES`] bytes at
//!    r8 through the storage write's translation, and returns the
//!    translation's status [OFW-4.93 lv1 a:0x259444-0x259460]. The
//!    request then keeps its tag, and r4 holds it.
//! 5. It takes the record's event mask at +48
//!    [OFW-4.93 lv1 a:0x259464]. The mask selects the path below.
//!
//! The paths, in the method's order:
//! - With [`NOTIFY_READ_TYPE0_EVENT_BIT`] set, while the record's index
//!   at +56 is below the bus's count at +3784, the method reports a
//!   type-0 event [OFW-4.93 lv1 a:0x25946C-0x259520].
//! - With [`NOTIFY_READ_DEVICE_EVENT_BIT`] set, while the record's index
//!   at +64 is below the bus's device-ready count at +3792, the method
//!   reports the device of the table entry at that index; see below
//!   [OFW-4.93 lv1 a:0x259524-0x2595E8].
//! - With [`NOTIFY_READ_QUEUED_EVENT_BIT`] set, while the record's queue
//!   at +1392 holds an event, the method reports that event
//!   [OFW-4.93 lv1 a:0x2595EC-0x2596E4].
//! - Otherwise the request waits. The method stores the buffer's real
//!   address at +72 of the record and the tag at +80, and returns 0
//!   [OFW-4.93 lv1 a:0x259704-0x259714].
//!
//! The device report does these steps:
//! 1. For a record whose partition is not 1, it runs lv1's storage device
//!    registration of the entry's device; see
//!    [`super::storage_registration`]
//!    [OFW-4.93 lv1 a:0x259540-0x25955C].
//! 2. It writes [`NOTIFY_READ_DEVICE_EVENT`] at +0 of the buffer, and the
//!    four doublewords of the 32-byte table entry at +3592 of the bus
//!    object from [`NOTIFY_READ_ENTRY_OFFSET`]
//!    [OFW-4.93 lv1 a:0x259560-0x2595CC]. See
//!    [`super::storage_registration::StorageDevice::notify_entry`].
//! 3. It stores 0 at +72 of the record, and adds 1 to the index at +64
//!    [OFW-4.93 lv1 a:0x25957C] [OFW-4.93 lv1 a:0x2595D0-0x2595D8].
//! 4. It completes the request with [`NOTIFY_READ_COMPLETION_STATUS`]
//!    through the storage write's completion function, which also sends
//!    the event on the partition's port, and returns 0
//!    [OFW-4.93 lv1 a:0x259588] [OFW-4.93 lv1 a:0x2595DC-0x2595E4].
//!
//! The open of the bus makes the record with 0 at +48, +56 and +64
//! [OFW-4.93 lv1 a:0x259208-0x259214], and with 0 at +72 and +80
//! [OFW-4.93 lv1 a:0x25911C-0x259144].
//!
//! The 4.93 kernel's call [OFW-4.93 lv2_kernel a:0x80000000000ae408]
//! passes these inputs
//! [OFW-4.93 lv2_kernel a:0x80000000000ae3ec-0x80000000000ae404]:
//! - r3 = -1;
//! - r4 = 0;
//! - r5 = 0;
//! - r6 = 1;
//! - r7 = 0;
//! - r8 = the buffer.

use super::status::{LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY, LV1_WRONG_STATE};

/// The handler of hypercall 245 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageReadHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `storage_read` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 245 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const STORAGE_READ_HANDLERS: &[StorageReadHandler] = &[StorageReadHandler {
    firmware: "4.93",
    handler: 0x30_8B88,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn storage_read_handler(firmware: &str) -> Option<&'static StorageReadHandler> {
    STORAGE_READ_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The status of a read of a device that does not exist:
/// `LV1_NO_ENTRY` (-6).
///
/// The read core returns it at [OFW-4.93 lv1 a:0x2515C0] in these cases:
/// - the device id is above
///   [`super::storage::STORAGE_DEVICE_ID_MAX`] and is not
///   [`super::storage::STORAGE_NOTIFICATION_DEVICE_ID`];
/// - the device's object pointer is null.
pub const STORAGE_READ_NO_DEVICE_STATUS: i64 = LV1_NO_ENTRY;

/// The status of a read of a device that is not ready: `LV1_WRONG_STATE`
/// (-11).
///
/// The read core returns it at [OFW-4.93 lv1 a:0x251580] when slot 6 of
/// the device's virtual table answers false.
pub const STORAGE_READ_NOT_READY_STATUS: i64 = LV1_WRONG_STATE;

/// The start sector that the notification read method accepts.
///
/// The method compares r6, the call's r5, with 0
/// [OFW-4.93 lv1 a:0x25934C].
pub const NOTIFY_READ_START_SECTOR: u64 = 0;

/// The sector count that the notification read method accepts.
///
/// The method compares r7, the call's r6, with 1
/// [OFW-4.93 lv1 a:0x259354].
pub const NOTIFY_READ_SECTOR_COUNT: u64 = 1;

/// The flags that the notification read method accepts.
///
/// The method compares r8, the call's r7, with 0
/// [OFW-4.93 lv1 a:0x25935C].
pub const NOTIFY_READ_FLAGS: u64 = 0;

/// The status of a notification read whose start sector, sector count or
/// flags the method does not accept: `LV1_ILLEGAL_PARAMETER_VALUE` (-17)
/// [OFW-4.93 lv1 a:0x2593DC].
pub const NOTIFY_READ_BAD_REQUEST_STATUS: i64 = LV1_ILLEGAL_PARAMETER_VALUE;

/// The status of a notification read from a partition that no open
/// record holds: `LV1_NO_ENTRY` (-6) [OFW-4.93 lv1 a:0x259394].
pub const NOTIFY_READ_NO_RECORD_STATUS: i64 = LV1_NO_ENTRY;

/// The event mask bit of type-0 events [OFW-4.93 lv1 a:0x25946C].
pub const NOTIFY_READ_TYPE0_EVENT_BIT: u64 = 1;

/// The event mask bit of device reports [OFW-4.93 lv1 a:0x259524].
pub const NOTIFY_READ_DEVICE_EVENT_BIT: u64 = 2;

/// The event mask bit of queued events [OFW-4.93 lv1 a:0x2595EC].
pub const NOTIFY_READ_QUEUED_EVENT_BIT: u64 = 4;

/// The doubleword at +0 of the buffer of a device report
/// [OFW-4.93 lv1 a:0x259560] [OFW-4.93 lv1 a:0x259568].
pub const NOTIFY_READ_DEVICE_EVENT: u64 = 1;

/// The offset in the buffer of a device report's table entry, whose four
/// doublewords end at +40 [OFW-4.93 lv1 a:0x259590]
/// [OFW-4.93 lv1 a:0x2595CC].
pub const NOTIFY_READ_ENTRY_OFFSET: u64 = 8;

/// The status with which a device report completes its request
/// [OFW-4.93 lv1 a:0x259588].
pub const NOTIFY_READ_COMPLETION_STATUS: u64 = 0;

#[cfg(test)]
#[path = "tests/storage_read_tests.rs"]
mod tests;
