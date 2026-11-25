//! lv1's `storage_write` (hypercall 246): the statuses of the writes it
//! refuses, and its notification write to device -1.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and
//! read with `cellgov dev disasm`, and from the console capture
//! [`NOTIFY_REQUEST_CAPTURE`]. The repository holds no byte of lv1,
//! only these facts. The path of a write:
//! 1. The handler, at [OFW-4.93 lv1 a:0x308A08] in the live table,
//!    zeroes the return slot of r4. It calls the storage write core at
//!    [OFW-4.93 lv1 a:0x251340] with the caller's partition, r3 to r8 and
//!    the slot's address [OFW-4.93 lv1 a:0x308AB4-0x308AF0].
//! 2. The core checks the device id and the device's state. Then it
//!    calls the device's write method, slot 33 of its virtual table, and
//!    returns that method's status [OFW-4.93 lv1 a:0x251470].
//! 3. Seven storage device classes share the write method
//!    at [OFW-4.93 lv1 a:0x254CB4]. It checks the flags, the flash alignment and the
//!    region. Then it calls the region access check at [OFW-4.93 lv1 a:0x25B5AC].
//! 4. For device id -1 [OFW-4.93 lv1 a:0x2514C0], the core uses the
//!    storage bus object as the device. The object's class has the
//!    virtual table [OFW-4.93 lv1 a:0x35DD30], and slot 33 of that table
//!    is the notification write method [OFW-4.93 lv1 a:0x25A664].
//! 5. The handler returns the core's r3 zero-extended
//!    [OFW-4.93 lv1 a:0x308B58].
//!
//! No read of the image covers these parts of the path:
//! - the order in which the shared write method makes its checks;
//! - the body of the region access check's ACL comparison, from
//!   [OFW-4.93 lv1 a:0x25B6F0-0x25B7C8].
//!
//! Each status below is the low word of r3, as an LV1 status code.
//!
//! # The notification write
//!
//! For device -1 the core and the notification write method do these
//! steps. The first failure is the answer:
//! 1. The core asks the storage bus object's ready method, slot 6
//!    [OFW-4.93 lv1 a:0x2513B8-0x2513CC]. The method returns true when one of
//!    the bus's two open slots holds the caller's partition and has its
//!    used byte set [OFW-4.93 lv1 a:0x25477C-0x2547FC]. A partition that
//!    did not open the bus gets [`STORAGE_WRITE_NOT_READY_STATUS`].
//! 2. The method refuses a start sector, a sector count or flags other
//!    than [`NOTIFY_WRITE_START_SECTOR`], [`NOTIFY_WRITE_SECTOR_COUNT`]
//!    and [`NOTIFY_WRITE_FLAGS`] with [`NOTIFY_WRITE_BAD_REQUEST_STATUS`]
//!    [OFW-4.93 lv1 a:0x25A664-0x25A6B8]. It does not read the region id.
//! 3. It translates the [`NOTIFY_BUFFER_BYTES`] bytes at r8
//!    [OFW-4.93 lv1 a:0x25A6F0-0x25A708]. The translation refuses an
//!    address outside the partition's memory, and a buffer that ends
//!    past its memory region [OFW-4.93 lv1 a:0x254484-0x2545A0]. The
//!    method returns the translation's status
//!    [OFW-4.93 lv1 a:0x25A700-0x25A708].
//! 4. It refuses a doubleword at +0 of the buffer other than
//!    [`NOTIFY_WRITE_COMMAND`] with [`NOTIFY_WRITE_BAD_REQUEST_STATUS`].
//!    It keeps the doubleword at [`NOTIFY_EVENT_MASK_OFFSET`] as the
//!    event mask [OFW-4.93 lv1 a:0x25A70C-0x25A720].
//! 5. For a partition other than 1, it stores the event mask at +48 of
//!    the partition's open record [OFW-4.93 lv1 a:0x25A728-0x25A7AC]
//!    [OFW-4.93 lv1 a:0x25A8CC-0x25A8D4]. For partition 1 it stores 5
//!    there [OFW-4.93 lv1 a:0x25A8D8-0x25A8E0]. Before the store it asks
//!    the ready method again, and returns -6 when the answer is false
//!    [OFW-4.93 lv1 a:0x25A75C-0x25A77C]. It returns -32768 when no
//!    record in the bus's list holds the partition
//!    [OFW-4.93 lv1 a:0x25A7B0].
//! 6. Its request function gives the request the lowest of
//!    [`NOTIFY_REQUEST_TAGS`] tags that no request holds, and stores the
//!    tag in the return slot of r4. When requests hold every tag, it returns
//!    [`NOTIFY_REQUESTS_FULL_STATUS`]
//!    [OFW-4.93 lv1 a:0x25398C-0x253A04].
//! 7. Its completion function completes the request with status
//!    [`NOTIFY_COMPLETION_STATUS`] while the bus is in
//!    [`STORAGE_BUS_COMPLETING_STATE`]
//!    [OFW-4.93 lv1 a:0x25A82C-0x25A838]
//!    [OFW-4.93 lv1 a:0x256464-0x256494]. For a port in the partition's
//!    slot that holds a reference, it then calls the event function with
//!    that port [OFW-4.93 lv1 a:0x256498-0x25654C].
//! 8. The method returns 0.
//!
//! The 4.93 kernel's call [OFW-4.93 lv2_kernel a:0x80000000000ae250]
//! passes these inputs
//! [OFW-4.93 lv2_kernel a:0x80000000000ae1dc-0x80000000000ae24c]:
//! - r3 = -1;
//! - r4 = 0;
//! - r5 = 0;
//! - r6 = 1;
//! - r7 = 0;
//! - a buffer with 0 at +0 and 2 at +8.
//!
//! On r3 = 0 it keeps r4 as the request's tag
//! [OFW-4.93 lv2_kernel a:0x80000000000ae254-0x80000000000ae278].

use super::status::{
    LV1_BUSY, LV1_DENIED_BY_POLICY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY, LV1_NO_PRIVILEGE,
    LV1_WRONG_STATE,
};

/// The handler of hypercall 246 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageWriteHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `storage_write` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 246 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const STORAGE_WRITE_HANDLERS: &[StorageWriteHandler] = &[StorageWriteHandler {
    firmware: "4.93",
    handler: 0x30_8A08,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn storage_write_handler(firmware: &str) -> Option<&'static StorageWriteHandler> {
    STORAGE_WRITE_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The console capture of the notification requests that lv1 holds on
/// the storage bus, and of the bus's state.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const NOTIFY_REQUEST_CAPTURE: &str = "tests/micro/lv1_storage_notify_request";

/// The largest storage device id: lv1 keeps one object pointer for each
/// device from 0 to 6.
///
/// [PS3DEVWIKI-HV-RE s:Storage_bus_subsystem] The storage bus object holds a table of pointers to at most 7 storage device objects at offset 0xEE8.
///
/// The lookup for the storage bus reads the table up to this id, with an
/// unsigned compare [OFW-4.93 lv1 a:0x2365E4-0x2365EC]
/// [OFW-4.93 lv1 a:0x236604-0x23661C].
pub const STORAGE_DEVICE_ID_MAX: u64 = 6;

/// The device id of the notification pseudo-device, -1.
///
/// The write core takes a separate path for it.
pub const STORAGE_NOTIFICATION_DEVICE_ID: u64 = u64::MAX;

/// The status of a write to a device that does not exist:
/// `LV1_NO_ENTRY` (-6).
///
/// The write core returns it, at [OFW-4.93 lv1 a:0x251438], in these cases:
/// - the device id is above [`STORAGE_DEVICE_ID_MAX`] and is not
///   [`STORAGE_NOTIFICATION_DEVICE_ID`];
/// - the device's object pointer, at offset 0xEE8 of the storage bus
///   object, is null.
pub const STORAGE_WRITE_NO_DEVICE_STATUS: i64 = LV1_NO_ENTRY;

/// The status of a write to a device that is not ready:
/// `LV1_WRONG_STATE` (-11). The write core asks the device through slot
/// 6 of its virtual table, and returns this status at [OFW-4.93 lv1 a:0x2513F8] when the
/// answer is false.
pub const STORAGE_WRITE_NOT_READY_STATUS: i64 = LV1_WRONG_STATE;

/// The flag bits that the shared write method refuses.
pub const STORAGE_WRITE_RESERVED_FLAGS: u64 = 0xFFFF_FFF8_00FF_FFC1;

/// The status of a write whose flags set a bit of
/// [`STORAGE_WRITE_RESERVED_FLAGS`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17).
///
/// The shared write method returns it at [OFW-4.93 lv1 a:0x254EE0].
pub const STORAGE_WRITE_BAD_FLAGS_STATUS: i64 = LV1_ILLEGAL_PARAMETER_VALUE;

/// The device type of a flash device, which the shared write method
/// reads through slot 37 of the device's virtual table.
///
/// Linux `arch/powerpc/platforms/ps3/platform.h` names it
/// `PS3_DEV_TYPE_STOR_FLASH`.
pub const STORAGE_FLASH_DEVICE_TYPE: u64 = 14;

/// The bits of a flash write's start sector and sector count that must
/// be 0.
pub const STORAGE_FLASH_ALIGNMENT_MASK: u64 = 0x1FF;

/// The status of a flash write whose start sector or sector count sets a
/// bit of [`STORAGE_FLASH_ALIGNMENT_MASK`]: -17.
///
/// The shared write method
/// makes this check from [OFW-4.93 lv1 a:0x254EA4-0x254EDC].
pub const STORAGE_WRITE_FLASH_ALIGNMENT_STATUS: i64 = LV1_ILLEGAL_PARAMETER_VALUE;

/// The status of a write to a region id that the device's region list
/// does not hold: -6.
///
/// The shared write method returns it at
/// [OFW-4.93 lv1 a:0x254DC0] and [OFW-4.93 lv1 a:0x254E4C]. The region
/// access check also returns it, at [OFW-4.93 lv1 a:0x25B6C8], when no
/// region matches.
pub const STORAGE_WRITE_NO_REGION_STATUS: i64 = LV1_NO_ENTRY;

/// The flag bits that skip the region access check.
///
/// With either bit set, the shared write method searches for the
/// partition instead
/// [OFW-4.93 lv1 a:0x254D2C], and returns [`STORAGE_WRITE_DENIED_STATUS`] when the
/// lookup fails.
///
/// [PS3DEVWIKI-HV-RE s:Region_Access_Protection] When flags & 0x100000002 is not 0, lv1 skips the access rights check.
pub const STORAGE_WRITE_SKIP_ACCESS_CHECK_FLAGS: u64 = 0x1_0000_0002;

/// The status of a write to a region that the partition may not write:
/// `LV1_DENIED_BY_POLICY` (-4).
///
/// The region access check at [OFW-4.93 lv1 a:0x25B5AC] returns it from
/// [OFW-4.93 lv1 a:0x25B79C] in these cases:
/// - the requested access, the flags with bit 0x2 set, shares no bit
///   with the region ACL's rights [OFW-4.93 lv1 a:0x25B7C8] [OFW-4.93 lv1 a:0x25B7D0];
/// - the partition lookup fails [OFW-4.93 lv1 a:0x25B608];
/// - the device's region list is empty [OFW-4.93 lv1 a:0x25B690].
///
/// A partition id of 0 or 1 passes the check [OFW-4.93 lv1 a:0x25B5F0].
///
/// [PS3DEVWIKI-HV-RE s:Region_Access_Protection] A storage access that the region ACLs deny returns LV1_DENIED_BY_POLICY.
pub const STORAGE_WRITE_DENIED_STATUS: i64 = LV1_DENIED_BY_POLICY;

/// The start sector that the notification write method accepts.
///
/// The method compares r6, the call's r5, with 0
/// [OFW-4.93 lv1 a:0x25A664] [OFW-4.93 lv1 a:0x25A6A4].
pub const NOTIFY_WRITE_START_SECTOR: u64 = 0;

/// The sector count that the notification write method accepts.
///
/// The method compares r7, the call's r6, with 1
/// [OFW-4.93 lv1 a:0x25A6A8-0x25A6AC].
pub const NOTIFY_WRITE_SECTOR_COUNT: u64 = 1;

/// The flags that the notification write method accepts.
///
/// The method compares r8, the call's r7, with 0
/// [OFW-4.93 lv1 a:0x25A6B0-0x25A6B4].
pub const NOTIFY_WRITE_FLAGS: u64 = 0;

/// The status of a notification write that the method refuses:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17).
///
/// The method returns it at [OFW-4.93 lv1 a:0x25A6B8] in these cases:
/// - a start sector, a sector count or flags that the method does not
///   accept;
/// - a buffer whose doubleword at +0 is not [`NOTIFY_WRITE_COMMAND`]
///   [OFW-4.93 lv1 a:0x25A714-0x25A718].
pub const NOTIFY_WRITE_BAD_REQUEST_STATUS: i64 = LV1_ILLEGAL_PARAMETER_VALUE;

/// The bytes of the buffer that the notification write method
/// translates [OFW-4.93 lv1 a:0x25A6F4].
pub const NOTIFY_BUFFER_BYTES: u64 = 512;

/// The status of a notification write whose [`NOTIFY_BUFFER_BYTES`]
/// bytes end past the memory region that holds their first byte:
/// `LV1_NO_PRIVILEGE` (-3).
///
/// The translation compares the bytes left in the region with the length
/// [OFW-4.93 lv1 a:0x254520-0x254538] and returns -3
/// [OFW-4.93 lv1 a:0x254500]. The method returns that status
/// [OFW-4.93 lv1 a:0x25A700-0x25A708].
pub const NOTIFY_WRITE_BUFFER_PAST_REGION_STATUS: i64 = LV1_NO_PRIVILEGE;

/// The doubleword at +0 of the buffer that the notification write
/// method accepts [OFW-4.93 lv1 a:0x25A70C-0x25A718].
pub const NOTIFY_WRITE_COMMAND: u64 = 0;

/// The offset in the buffer of the event mask, which the method stores
/// in the partition's open record [OFW-4.93 lv1 a:0x25A720]
/// [OFW-4.93 lv1 a:0x25A8D0].
pub const NOTIFY_EVENT_MASK_OFFSET: u64 = 8;

/// The tags of the storage bus's notification requests.
///
/// The request function keeps one bit for each tag at +2680 of the bus
/// object. It refuses a request when that doubleword is 0xFFFF, so each
/// tag is from 0 to 15 [OFW-4.93 lv1 a:0x25398C-0x2539CC].
pub const NOTIFY_REQUEST_TAGS: u8 = 16;

/// The status of a notification request when requests hold every tag:
/// `LV1_BUSY` (-9) [OFW-4.93 lv1 a:0x2539C8].
pub const NOTIFY_REQUESTS_FULL_STATUS: i64 = LV1_BUSY;

/// The status with which the notification write completes its request
/// [OFW-4.93 lv1 a:0x25A834].
pub const NOTIFY_COMPLETION_STATUS: u64 = 0;

/// The state of the storage bus in which the completion function
/// completes a request in the call: 4, the word at +3456 of the bus
/// object [OFW-4.93 lv1 a:0x256464-0x256470].
///
/// The capture [`NOTIFY_REQUEST_CAPTURE`] reads this state after boot.
pub const STORAGE_BUS_COMPLETING_STATE: u32 = 4;

/// The doubleword at +3464 of the storage bus object, which the request
/// function gives each new request [OFW-4.93 lv1 a:0x25A7CC]: 0, from
/// the capture [`NOTIFY_REQUEST_CAPTURE`].
///
/// For a doubleword of 0 the request takes no memory object
/// [OFW-4.93 lv1 a:0x2539D0-0x2539D8].
pub const NOTIFY_REQUEST_WORD: u64 = 0;

/// The tags of the notification requests that partition 1 holds at the
/// 4.93 kernel's call: tag 0, one bit for each tag.
///
/// The capture [`NOTIFY_REQUEST_CAPTURE`] reads tag 0 as partition 1's
/// request after boot. Partition 1 opened the storage bus before the
/// game OS partition, and holds a request from its first request on.
/// The capture cannot show the bitmap at the time of the call: a
/// completion of partition 1's request at that time leaves tag 0 free.
pub const OTHER_PARTITION_NOTIFY_TAGS: u16 = 0b1;

#[cfg(test)]
#[path = "tests/storage_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/storage_notify_tests.rs"]
mod notify_tests;

#[cfg(test)]
#[path = "tests/storage_notify_console_tests.rs"]
pub(crate) mod notify_console_tests;
