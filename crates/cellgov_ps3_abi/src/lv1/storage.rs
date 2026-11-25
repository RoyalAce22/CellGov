//! The statuses lv1 returns for a `storage_write` (hypercall 246) that
//! it refuses, read from the 4.93 lv1.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and
//! read with `cellgov dev disasm`. No byte of lv1 is committed; these
//! facts are. The path of a write:
//! 1. The handler, at [OFW-4.93 lv1 a:0x308A08] in the live table, calls the storage
//!    write core at [OFW-4.93 lv1 a:0x251340] and returns its r3.
//! 2. The core checks the device id and the device's state. Then it
//!    calls the device's write method, slot 33 of its virtual table, and
//!    returns that method's status [OFW-4.93 lv1 a:0x251470].
//! 3. Seven of the eight storage device classes share the write method
//!    at [OFW-4.93 lv1 a:0x254CB4]. It checks the flags, the flash alignment and the
//!    region. Then it calls the region access check at [OFW-4.93 lv1 a:0x25B5AC].
//!
//! These parts of the path were not read:
//! - the order in which the write method makes its checks;
//! - the write method of the eighth class (virtual table [OFW-4.93 lv1 a:0x35DD30],
//!   method [OFW-4.93 lv1 a:0x25A664]), so the status for a read-only device is not
//!   known;
//! - the body of the region access check's ACL comparison, from
//!   [OFW-4.93 lv1 a:0x25B6F0-0x25B7C8];
//! - the notification device's path, for device id -1 [OFW-4.93 lv1 a:0x2514C0];
//! - the width of r3: whether the hypercall returns these statuses
//!   sign-extended or zero-extended.
//!
//! Each status below is the low word of r3, as an LV1 status code.

use super::status::{
    LV1_DENIED_BY_POLICY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY, LV1_WRONG_STATE,
};

/// The largest storage device id: lv1 keeps one object pointer for each
/// device from 0 to 6.
///
/// [PS3DEVWIKI-HV-RE s:Storage_bus_subsystem] The storage bus object holds a table of pointers to at most 7 storage device objects at offset 0xEE8.
pub const STORAGE_DEVICE_ID_MAX: u64 = 6;

/// The device id of the notification pseudo-device, -1. The write core
/// sends it down a path of its own.
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
/// The shared write method returns it at [OFW-4.93 lv1 a:0x254EE0].
pub const STORAGE_WRITE_BAD_FLAGS_STATUS: i64 = LV1_ILLEGAL_PARAMETER_VALUE;

/// The device type of a flash device, which the shared write method
/// reads through slot 37 of the device's virtual table. Linux
/// `arch/powerpc/platforms/ps3/platform.h` names it `PS3_DEV_TYPE_STOR_FLASH`.
pub const STORAGE_FLASH_DEVICE_TYPE: u64 = 14;

/// The bits of a flash write's start sector and sector count that must
/// be 0.
pub const STORAGE_FLASH_ALIGNMENT_MASK: u64 = 0x1FF;

/// The status of a flash write whose start sector or sector count sets a
/// bit of [`STORAGE_FLASH_ALIGNMENT_MASK`]: -17. The shared write method
/// makes this check from [OFW-4.93 lv1 a:0x254EA4-0x254EDC].
pub const STORAGE_WRITE_FLASH_ALIGNMENT_STATUS: i64 = LV1_ILLEGAL_PARAMETER_VALUE;

/// The status of a write to a region id that the device's region list
/// does not hold: -6. The shared write method returns it at
/// [OFW-4.93 lv1 a:0x254DC0] and [OFW-4.93 lv1 a:0x254E4C]. The region
/// access check also returns it, at [OFW-4.93 lv1 a:0x25B6C8], when no
/// region matches.
pub const STORAGE_WRITE_NO_REGION_STATUS: i64 = LV1_NO_ENTRY;

/// The flag bits that skip the region access check. With either bit
/// set, the shared write method looks the partition up instead
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

#[cfg(test)]
#[path = "tests/storage_tests.rs"]
mod tests;
