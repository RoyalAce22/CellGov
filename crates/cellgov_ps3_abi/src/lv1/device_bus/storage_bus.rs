//! The facts of `open_device` for the storage bus and its devices.
//!
//! # The storage bus
//!
//! The open function sends bus 4 to the lookup
//! [OFW-4.93 lv1 a:0x237714-0x237718]. The lookup for bus 4
//! [OFW-4.93 lv1 a:0x2365B0-0x2365B4] reads the storage bus object from
//! the global that the TOC slot at r2 - 26656 locates
//! [OFW-4.93 lv1 a:0x2365E0-0x2365E8]. The slot holds 0x368388
//! [OFW-4.93 lv1 a:0x353818-0x35381C]. The lookup returns one of these:
//! - for a device id up to 6, the object's doubleword at
//!   3816 + 8 x device [OFW-4.93 lv1 a:0x236604-0x23661C];
//! - for [`STORAGE_BUS_DEVICE`], the storage bus object itself
//!   [OFW-4.93 lv1 a:0x2365F0-0x2365FC];
//! - for any other device id, 0 [OFW-4.93 lv1 a:0x2365F4-0x2365F8].
//!
//! The last case reads nothing of the object. So for such a device id the
//! open function always returns [`OPEN_DEVICE_NO_DEVICE_R3`]; see
//! [`storage_bus_device_absent`].
//!
//! For a bus other than 1, the open function calls the method at +24 of
//! the object's method table with the object, the caller's partition and
//! the call's r5. It returns that method's r3 zero-extended
//! [OFW-4.93 lv1 a:0x23760C-0x237630].
//!
//! The object's constructor [OFW-4.93 lv1 a:0x259280] does these steps:
//! - It stores the method table at +0 [OFW-4.93 lv1 a:0x259298]
//!   [OFW-4.93 lv1 a:0x2592AC]. The TOC slot that it reads holds 0x35DD30
//!   [OFW-4.93 lv1 a:0x354670-0x354674].
//! - It sets the byte at +3496 to 1 [OFW-4.93 lv1 a:0x25929C-0x2592A4].
//! - It clears the lock word at +3576 and the record list at +3552
//!   [OFW-4.93 lv1 a:0x2592BC-0x2592DC].
//!
//! The method at +24 is the open method [OFW-4.93 lv1 a:0x35DD48-0x35DD4C]
//! [OFW-4.93 lv1 a:0x335568-0x33556C]. It holds the lock at +3576
//! [OFW-4.93 lv1 a:0x259070-0x259094] and calls the partition check
//! [OFW-4.93 lv1 a:0x2545A4] [OFW-4.93 lv1 a:0x259098-0x2590A0]. Then it
//! does one of these steps:
//! - When the check returns a nonzero status, the method returns that
//!   status sign-extended [OFW-4.93 lv1 a:0x2590A4-0x2590C4].
//! - Otherwise it allocates a 1416-byte record. When the allocation
//!   fails, it returns -2 [OFW-4.93 lv1 a:0x2590E8-0x2590F8]
//!   [OFW-4.93 lv1 a:0x259278]. The slot that the check took stays taken.
//! - Otherwise it fills the record with sixteen 80-byte entries from +88
//!   and a list of them at +1368 [OFW-4.93 lv1 a:0x2590FC-0x2591FC]. It
//!   keeps the partition at +40 of the record, links the record into the
//!   list at +3552 and returns 0 [OFW-4.93 lv1 a:0x259204-0x259258].
//!
//! The partition check does these steps:
//! 1. It reads lpar 1's `ss.laid` node for the partition
//!    [OFW-4.93 lv1 a:0x2545B8-0x2545D0] [OFW-4.93 lv1 a:0x31CE9C-0x31CEF8].
//!    A failed read gives -4 [OFW-4.93 lv1 a:0x2546EC-0x2546F8].
//! 2. It tests the byte at +3496 and the partition's authority id
//!    [OFW-4.93 lv1 a:0x2545DC-0x2545F4] [OFW-4.93 lv1 a:0x254694-0x2546E8]
//!    [OFW-4.93 lv1 a:0x25473C-0x25475C]. A failed test gives -6 or -4.
//! 3. It keeps the partition at +112 and sets the byte at +108
//!    [OFW-4.93 lv1 a:0x2545F8-0x254600].
//! 4. It holds the lock at +2536 and tests two 64-byte slots, at +2544 and
//!    then at +2608 [OFW-4.93 lv1 a:0x254604-0x254660]. A free slot ends
//!    the test, so it does not read a later slot. A used slot that holds
//!    the caller's partition gives -9 [OFW-4.93 lv1 a:0x254734]. Two used
//!    slots that hold other partitions give -2 [OFW-4.93 lv1 a:0x254658].
//! 5. Otherwise it takes the free slot: it keeps the partition at +0, the
//!    call's r5 at +8 and sets the byte at +16. It returns 0
//!    [OFW-4.93 lv1 a:0x254710-0x25472C] [OFW-4.93 lv1 a:0x254664-0x254678].
//!
//! So when the first slot is free, a partition that holds the second slot
//! takes the first slot too, and gets 0.
//!
//! Later lv1 paths test the sign of the r5 that the slot keeps
//! [OFW-4.93 lv1 a:0x254884-0x25488C] [OFW-4.93 lv1 a:0x255D30-0x255D38].
//!
//! The 4.93 kernel opens the storage bus with r3 = 4, r4 = -1 and r5 = 0,
//! and continues only when r3 is 0
//! [OFW-4.93 lv2_kernel a:0x80000000000ae100-0x80000000000ae118]. The
//! capture [`STORAGE_BUS_OPEN_CAPTURE`] shows that lv1 holds the storage
//! bus open for the game OS partition after the boot. The byte at +3496
//! is 1, and the partition's slot keeps r5 = 0. The partition's record is
//! in the list. So the checks passed and the allocation succeeded for the
//! kernel's open. The first slot holds partition 1, so a second open of
//! the game OS partition gets -9.
//!
//! CellGov keeps only the r5 of the game OS partition's open. It does not
//! model the object's +108 and +112, the slot of partition 1, or the
//! record and its entries.
//!
//! # Devices 0 to 6 of the storage bus
//!
//! For a device id up to 6, the lookup returns the device object D at
//! +3816 + 8 x id of the storage bus object
//! [OFW-4.93 lv1 a:0x236604-0x23661C]. The open function then calls the
//! method at +24 of D's method table with D, the caller's partition and
//! the call's r5. It returns that method's r3 zero-extended
//! [OFW-4.93 lv1 a:0x23760C-0x237630].
//!
//! The capture [`STORAGE_DEVICE_OPEN_CAPTURE`] reads the method table of
//! each of the five devices of the 4.93 bus table. The entry at +24 of
//! each table holds the descriptor 0x334BF0:
//! - device 0, table 0x35E518 [OFW-4.93 lv1 a:0x35E530-0x35E534];
//! - device 1, table 0x35E6E8 [OFW-4.93 lv1 a:0x35E700-0x35E704];
//! - device 2, table 0x35E370 [OFW-4.93 lv1 a:0x35E388-0x35E38C];
//! - device 3, table 0x35E1C8 [OFW-4.93 lv1 a:0x35E1E0-0x35E1E4];
//! - device 4, table 0x35DFF8 [OFW-4.93 lv1 a:0x35E010-0x35E014].
//!
//! The descriptor's entry is the partition check of the storage bus
//! [OFW-4.93 lv1 a:0x334BF0-0x334BF4]. So the open of a storage device is
//! that check on D, with no record after it. The check returns its status
//! sign-extended [OFW-4.93 lv1 a:0x254678], and the open function clears
//! the high word.
//!
//! The check's second step passes for D when these tests pass:
//! - D's byte at +3496 is set [OFW-4.93 lv1 a:0x2545DC-0x2545E4];
//! - the doubleword at +2352 of the object at D + 144 is not 0, or the
//!   method at +296 of D's table returns a value that is not 0
//!   [OFW-4.93 lv1 a:0x2545E8-0x2545F4] [OFW-4.93 lv1 a:0x254694-0x2546B4].
//!
//! When either test fails, the partition's authority id decides. A
//! failed second test passes for two authority ids
//! [OFW-4.93 lv1 a:0x2546B8-0x2546E8]. A clear byte at +3496 lets one
//! authority id go on to the second test, and the check returns -6 for
//! each other id [OFW-4.93 lv1 a:0x25473C-0x25475C].
//!
//! The capture reads these values after the boot to the XMB:
//! - Each device's byte at +3496 is 1, its byte at +108 is 1 and its
//!   +112 holds partition 2.
//! - For devices 0, 2 and 3, slot 0 holds partition 2 with r5 =
//!   0x8000000000000000, and slot 1 is free.
//! - For devices 1 and 4, slot 0 holds partition 1 with r5 = 0, and
//!   slot 1 holds partition 2 with r5 = 0x8000000000000000.
//!
//! The check writes a slot only on the path that returns 0. So the game
//! OS partition opened each device with r5 = 0x8000000000000000, and the
//! check passed for each open. A second open of the game OS partition
//! finds its own slot, and gets -9.
//!
//! The 4.93 kernel's device object opens the device whose id is at +152
//! of the object, on the bus whose id is at +144, with
//! r5 = 0x8000000000000000
//! [OFW-4.93 lv2_kernel a:0x80000000000ac0fc-0x80000000000ac120]. The
//! object's constructor copies the bus id and the device id from +0 and
//! +8 of the kernel's device record
//! [OFW-4.93 lv2_kernel a:0x80000000000a98c8-0x80000000000a98e8].
//!
//! CellGov keeps only the device id and the r5 of each game OS open. It
//! does not model the slot of partition 1 or D's +108 and +112.
//!
//! [`OPEN_DEVICE_NO_DEVICE_R3`]: super::OPEN_DEVICE_NO_DEVICE_R3

use crate::lv1::status::{zero_extended, LV1_BUSY};
use crate::lv1::storage::STORAGE_DEVICE_ID_MAX;
use crate::lv1::storage_registration::registration;

/// The bus id of the storage bus, which the lookup reads from the storage
/// bus object [OFW-4.93 lv1 a:0x2365B0-0x2365B4].
pub const STORAGE_BUS: u64 = 4;

/// The device id that names the storage bus object itself, -1
/// [OFW-4.93 lv1 a:0x2365F0-0x2365FC].
pub const STORAGE_BUS_DEVICE: u64 = u64::MAX;

/// Whether `open_device` of device `device` on bus `bus` opens the storage
/// bus object.
pub const fn opens_storage_bus(bus: u64, device: u64) -> bool {
    bus == STORAGE_BUS && device == STORAGE_BUS_DEVICE
}

/// Whether the lookup finds no device for device `device` of bus `bus`
/// whatever the storage bus object holds.
///
/// For bus [`STORAGE_BUS`], a device id above
/// [`STORAGE_DEVICE_ID_MAX`] other than [`STORAGE_BUS_DEVICE`]
/// gets 0 from the lookup [OFW-4.93 lv1 a:0x2365F0-0x2365F8]. The open
/// function then returns [`OPEN_DEVICE_NO_DEVICE_R3`]
/// [OFW-4.93 lv1 a:0x237600] [OFW-4.93 lv1 a:0x237780-0x237790].
///
/// [`OPEN_DEVICE_NO_DEVICE_R3`]: super::OPEN_DEVICE_NO_DEVICE_R3
pub const fn storage_bus_device_absent(bus: u64, device: u64) -> bool {
    bus == STORAGE_BUS && device > STORAGE_DEVICE_ID_MAX && device != STORAGE_BUS_DEVICE
}

/// The r3 of an open of the storage bus that the partition holds open:
/// `LV1_BUSY` (-9), zero-extended.
///
/// The partition check returns -9 [OFW-4.93 lv1 a:0x254734]. The open
/// method sign-extends it [OFW-4.93 lv1 a:0x2590C4], and the open function
/// clears the high word [OFW-4.93 lv1 a:0x237630].
pub const OPEN_STORAGE_BUS_BUSY_R3: u64 = zero_extended(LV1_BUSY);

/// The read-only lv1 capture of the storage bus object after the 4.93
/// boot, which shows the game OS partition's open of the storage bus.
pub const STORAGE_BUS_OPEN_CAPTURE: &str = "tests/micro/lv1_storage_bus_open";

/// The read-only lv1 capture of the storage device objects after the 4.93
/// boot, which shows the game OS partition's open of each device.
pub const STORAGE_DEVICE_OPEN_CAPTURE: &str = "tests/micro/lv1_storage_device_open";

/// Whether `open_device` of device `device` on bus `bus` at the firmware
/// version `firmware` opens a storage device whose open path CellGov read.
///
/// The read covers each device of the bus table of [`registration`] at
/// `firmware`. [`STORAGE_DEVICE_OPEN_CAPTURE`] reads the device object of
/// each 4.93 device.
pub fn opens_storage_device(firmware: &str, bus: u64, device: u64) -> bool {
    bus == STORAGE_BUS
        && device <= STORAGE_DEVICE_ID_MAX
        && registration(firmware).is_some_and(|table| table.device(device).is_some())
}

/// The r3 of an open of a storage device that the partition holds open:
/// `LV1_BUSY` (-9), zero-extended.
///
/// The partition check returns -9 [OFW-4.93 lv1 a:0x254734] sign-extended
/// [OFW-4.93 lv1 a:0x254678], and the open function clears the high word
/// [OFW-4.93 lv1 a:0x237630].
pub const OPEN_STORAGE_DEVICE_BUSY_R3: u64 = zero_extended(LV1_BUSY);

#[cfg(test)]
#[path = "tests/absent_device_tests.rs"]
mod absent_device_tests;
