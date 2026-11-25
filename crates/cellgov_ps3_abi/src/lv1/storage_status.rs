//! lv1's `storage_get_async_status` (hypercall 249): how lv1 gives the
//! partition a completed request of the storage bus's notification
//! device.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts. The path of a call:
//! 1. The handler, at [OFW-4.93 lv1 a:0x3093AC] in the live table,
//!    zeroes the return slots of r4 and r5, HSPRG0 - 0x6A38 and
//!    HSPRG0 - 0x6A30. It calls the status core with the caller's
//!    partition, r3 and the addresses of the two slots
//!    [OFW-4.93 lv1 a:0x309430-0x309460]. It returns the core's r3
//!    zero-extended [OFW-4.93 lv1 a:0x3094C8]. The vector's entry zeroes
//!    the other slots [OFW-4.93 lv1 a:0x0C08], so r6 to r10 return 0.
//! 2. The core [OFW-4.93 lv1 a:0x251E9C] reads the storage bus object
//!    from the global at 0x368388, as the write core does. For device -1
//!    it uses the bus object as the device
//!    [OFW-4.93 lv1 a:0x251FC0-0x251FC4]. It returns
//!    [`ASYNC_STATUS_NO_DEVICE_STATUS`] in these cases
//!    [OFW-4.93 lv1 a:0x251EAC-0x251EF0] [OFW-4.93 lv1 a:0x251F54-0x251F58]:
//!    - a device id above 6, other than -1;
//!    - a device whose object pointer is null.
//! 3. The core asks the device's ready method, slot 6 of its virtual
//!    table. A false answer gives [`ASYNC_STATUS_NOT_READY_STATUS`]
//!    [OFW-4.93 lv1 a:0x251EF4-0x251F30]. For the bus object, slot 6 of
//!    its virtual table [OFW-4.93 lv1 a:0x35DD60-0x35DD64] is the method
//!    at [OFW-4.93 lv1 a:0x25477C] [OFW-4.93 lv1 a:0x338778-0x33877C].
//!    The storage write's ready check asks the same slot
//!    [OFW-4.93 lv1 a:0x2513B8-0x2513CC].
//! 4. Otherwise the core calls slot 35 of the virtual table with the
//!    device, the partition, the two slot addresses and the kind
//!    [`ASYNC_STATUS_KIND`]. It returns that method's r3
//!    [OFW-4.93 lv1 a:0x251F0C-0x251F24] [OFW-4.93 lv1 a:0x251F80-0x251FA4].
//!    For the bus object, slot 35 is the status method at
//!    [OFW-4.93 lv1 a:0x255DF8] [OFW-4.93 lv1 a:0x35DE48-0x35DE4C]
//!    [OFW-4.93 lv1 a:0x334D40-0x334D44].
//!
//! # The status method of the storage bus
//!
//! The method holds the lock at +2672 of the bus object
//! [OFW-4.93 lv1 a:0x255E20-0x255E44]. It walks the 16 request entries
//! at +2688, 48 bytes each, from tag 0 [OFW-4.93 lv1 a:0x255E48-0x255E70].
//! It takes the first entry with these properties:
//! - the completed byte at +40 is set [OFW-4.93 lv1 a:0x255E74-0x255E7C];
//! - the partition at +0 is the caller's
//!   [OFW-4.93 lv1 a:0x255E80-0x255E8C];
//! - the kind byte at +41 is the core's kind
//!   [OFW-4.93 lv1 a:0x255E90-0x255E98].
//!
//! For that entry the method does these steps:
//! 1. It stores the tag in the slot of r4 and the status at +24 in the
//!    slot of r5 [OFW-4.93 lv1 a:0x255E9C-0x255EAC].
//! 2. When the tag's bit of the request bitmap at +2680 is set, it clears
//!    the bit. It releases the memory object at +8 when there is one, and
//!    zeroes +0, the kind byte and the completed byte
//!    [OFW-4.93 lv1 a:0x255EB4-0x255F00].
//! 3. It releases the lock. When the doubleword at +32
//!    [OFW-4.93 lv1 a:0x255EB8] is 0 it returns 0
//!    [OFW-4.93 lv1 a:0x255F04-0x255F18] [OFW-4.93 lv1 a:0x255FC4].
//!
//! When no entry matches, the method holds the lock at +2536 and tests
//! the open slot at +2544 and then the one at +2608 for the caller's
//! partition [OFW-4.93 lv1 a:0x255FCC-0x256008]. The slots are the two
//! slots of the open; see the storage bus section of
//! [`super::device_bus`]. For a slot with these properties, the method
//! takes the doubleword at +56 of the slot and leaves 0 there:
//! - the slot's used byte is set;
//! - the slot's open r5 sets [`ASYNC_STATUS_SLOT_WORD_FLAG`].
//!
//! A nonzero word gives r3 = 0, r4 = -1 and r5 = the word
//! [OFW-4.93 lv1 a:0x256054-0x25609C] [OFW-4.93 lv1 a:0x2560A0-0x2560D8].
//! When the first slot gives no word, the method then tests the second
//! slot. In every other case the method returns
//! [`ASYNC_STATUS_NONE_STATUS`] and writes no slot
//! [OFW-4.93 lv1 a:0x25600C-0x256028].
//!
//! The requests of the notification write match the walk:
//! - the write's request function stores the kind 0 at +41
//!   [OFW-4.93 lv1 a:0x25A808] [OFW-4.93 lv1 a:0x253A54-0x253A74];
//! - it stores 0 at +32 [OFW-4.93 lv1 a:0x253A48-0x253A50];
//! - the completion function writes only the status at +24 and the
//!   completed byte [OFW-4.93 lv1 a:0x256484-0x256494].
//!
//! # The kernel's call
//!
//! The 4.93 kernel's storage interrupt function sends hypercall 249 with
//! r3 = -1 [OFW-4.93 lv2_kernel a:0x80000000000ae51c-0x80000000000ae524].
//! On r3 = 0 it keeps r4 as the tag and r5 as the status, and sends the
//! call again [OFW-4.93 lv2_kernel a:0x80000000000ae528-0x80000000000ae544]
//! [OFW-4.93 lv2_kernel a:0x80000000000ae514-0x80000000000ae518]. For a
//! status of 0 it looks up the tag in its own table first
//! [OFW-4.93 lv2_kernel a:0x80000000000ae548-0x80000000000ae564]. A
//! nonzero r3 ends the loop [OFW-4.93 lv2_kernel a:0x80000000000ae5cc].

use super::status::{LV1_NO_ENTRY, LV1_WRONG_STATE};

/// The handler of hypercall 249 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageStatusHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `storage_get_async_status` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 249 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const STORAGE_STATUS_HANDLERS: &[StorageStatusHandler] = &[StorageStatusHandler {
    firmware: "4.93",
    handler: 0x30_93AC,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn storage_status_handler(firmware: &str) -> Option<&'static StorageStatusHandler> {
    STORAGE_STATUS_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The status of a call for a device that does not exist:
/// `LV1_NO_ENTRY` (-6) [OFW-4.93 lv1 a:0x251F58].
pub const ASYNC_STATUS_NO_DEVICE_STATUS: i64 = LV1_NO_ENTRY;

/// The status of a call for a device that is not ready for the partition:
/// `LV1_WRONG_STATE` (-11) [OFW-4.93 lv1 a:0x251F28].
pub const ASYNC_STATUS_NOT_READY_STATUS: i64 = LV1_WRONG_STATE;

/// The status of a call to the notification device when the partition
/// has no completed request and no slot word: `LV1_NO_ENTRY` (-6)
/// [OFW-4.93 lv1 a:0x25600C].
pub const ASYNC_STATUS_NONE_STATUS: i64 = LV1_NO_ENTRY;

/// The request kind that the core asks the status method for
/// [OFW-4.93 lv1 a:0x251F20].
pub const ASYNC_STATUS_KIND: u8 = 0;

/// The bit of the open's r5 with which the status method reads the slot
/// word: the sign bit, which the method tests with a signed compare
/// [OFW-4.93 lv1 a:0x256060-0x256068] [OFW-4.93 lv1 a:0x2560AC-0x2560B4].
pub const ASYNC_STATUS_SLOT_WORD_FLAG: u64 = 1 << 63;

#[cfg(test)]
#[path = "tests/storage_status_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/storage_status_console_tests.rs"]
mod console_tests;
