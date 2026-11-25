//! The facts of the device bus hypercalls: the devices, their MMIO and
//! DMA regions, the I/O segments and the IOMMU entries.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! # `open_device`
//!
//! The handler [OFW-4.93 lv1 a:0x305A38] does these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`OPEN_DEVICE_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x305AC8-0x305ACC].
//! 2. It calls the open function [OFW-4.93 lv1 a:0x2375B0] with the
//!    caller's partition, r3, r4 and r5 [OFW-4.93 lv1 a:0x305A4C-0x305A68]
//!    [OFW-4.93 lv1 a:0x305AD0-0x305AEC].
//! 3. It returns that function's r3 [OFW-4.93 lv1 a:0x305AF0]
//!    [OFW-4.93 lv1 a:0x305B58]. It writes no return slot, and the
//!    vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to
//!    r10 return 0.
//!
//! The open function does these steps:
//! 1. It refuses a bus id other than 1, 2, 3 and 4 with
//!    [`OPEN_DEVICE_NO_BUS_R3`] [OFW-4.93 lv1 a:0x2375C0-0x2375E8]
//!    [OFW-4.93 lv1 a:0x237714-0x237754].
//! 2. It looks up the device [OFW-4.93 lv1 a:0x2375EC-0x2375F4]. When the
//!    lookup finds no device, it returns [`OPEN_DEVICE_NO_DEVICE_R3`]
//!    [OFW-4.93 lv1 a:0x237600] [OFW-4.93 lv1 a:0x237780-0x237790].
//! 3. For bus 1, it takes a path of its own for each of the devices 0, 1
//!    and 2 [OFW-4.93 lv1 a:0x237604-0x237608]
//!    [OFW-4.93 lv1 a:0x237658-0x23765C] [OFW-4.93 lv1 a:0x237758-0x237764].
//!
//! # Device 0 of bus 1
//!
//! The lookup for bus 1 [OFW-4.93 lv1 a:0x236594-0x2365AC]
//! [OFW-4.93 lv1 a:0x236648-0x236658] reads device 0 from slot 0 of the
//! system bus object [OFW-4.93 lv1 a:0x235D88-0x235DA0]
//! [OFW-4.93 lv1 a:0x235DC8-0x235DE8] [OFW-4.93 lv1 a:0x22B838-0x22B844].
//! lv1's init fills that slot [OFW-4.93 lv1 a:0x3A9924-0x3A9948]. It then
//! gives the device two MMIO regions [OFW-4.93 lv1 a:0x3A993C-0x3A9968],
//! which are the regions that the 4.93 kernel maps after its open
//! [OFW-4.93 lv2_kernel a:0x800000000009370C-0x8000000000093734].
//!
//! The device's constructor sets its method table
//! [OFW-4.93 lv1 a:0x22DDC0-0x22DDD4] [OFW-4.93 lv1 a:0x353480-0x353484].
//! The table's open entry [OFW-4.93 lv1 a:0x35C890-0x35C894]
//! [OFW-4.93 lv1 a:0x32FA48-0x32FA4C] calls the table's entry at +16
//! [OFW-4.93 lv1 a:0x35C888-0x35C88C] [OFW-4.93 lv1 a:0x32D4F8-0x32D4FC],
//! and sign-extends its r3 [OFW-4.93 lv1 a:0x226058-0x22608C]. That entry
//! holds the device's lock and does these steps:
//! - When the device's open flag at +108 is set, it returns `LV1_BUSY`
//!   [OFW-4.93 lv1 a:0x2280A0-0x2280AC].
//! - Otherwise it keeps the caller's partition at +112, sets the open flag
//!   and returns 0 [OFW-4.93 lv1 a:0x2280B0-0x2280BC].
//!
//! No step reads r5 [OFW-4.93 lv1 a:0x228078-0x2280D4].
//!
//! The open function calls that entry with the bus 1 lock held
//! [OFW-4.93 lv1 a:0x237660-0x2376BC]. It returns a nonzero r3 of the
//! entry zero-extended [OFW-4.93 lv1 a:0x2376C0-0x2376C8]. After a 0, it runs the
//! hook [OFW-4.93 lv1 a:0x23E2F8] [OFW-4.93 lv1 a:0x237794-0x2377B0]. The
//! hook gives the global object at +16 a 288-byte object of its own and
//! stores to a register block [OFW-4.93 lv1 a:0x23E174-0x23E1F8]
//! [OFW-4.93 lv1 a:0x242860-0x2428F4]. When the hook fails, the function
//! closes the device again and returns the hook's status zero-extended
//! [OFW-4.93 lv1 a:0x2377B4-0x2377D8]. Otherwise it returns 0
//! [OFW-4.93 lv1 a:0x2377A8].
//!
//! The hook fails only on state of lv1's own:
//! - the object at +16 is absent [OFW-4.93 lv1 a:0x23E2F8-0x23E31C];
//! - an earlier hook already gave that object its state 2
//!   [OFW-4.93 lv1 a:0x23E33C-0x23E348];
//! - the 288-byte allocation fails [OFW-4.93 lv1 a:0x23E19C-0x23E1B8];
//! - the object's first byte is not 0 or 1
//!   [OFW-4.93 lv1 a:0x23E194-0x23E1A8] [OFW-4.93 lv1 a:0x242890-0x2428A4].
//!
//! No input of the call selects these cases. Another lv1 path runs the
//! same hook with the global object's field at +8
//! [OFW-4.93 lv1 a:0x247164-0x247170].
//!
//! The 4.93 kernel's call opens device 0 of bus 1 with r5 = 0
//! [OFW-4.93 lv2_kernel a:0x8000000000095704-0x8000000000095718]. Its
//! caller does not test the result
//! [OFW-4.93 lv2_kernel a:0x80000000000936F0-0x800000000009370C].
//!
//! # Device 1 of bus 1
//!
//! The lookup reads device 1 from slot 3 of the system bus object
//! [OFW-4.93 lv1 a:0x235DA4-0x235DAC] [OFW-4.93 lv1 a:0x235DC8-0x235DE8].
//! lv1's init fills that slot with the constructor of slot 0
//! [OFW-4.93 lv1 a:0x3A9A4C-0x3A9A70] [OFW-4.93 lv1 a:0x3A9758-0x3A9780].
//! The device's method table is therefore the table of device 0.
//!
//! The open function does not call the device's open entry for device 1
//! [OFW-4.93 lv1 a:0x237758-0x23775C]. It does these steps instead
//! [OFW-4.93 lv1 a:0x2377DC-0x2377F0]:
//! 1. It gets the controller object from slot 0 of an array
//!    [OFW-4.93 lv1 a:0x23D0F8-0x23D104]. lv1's init makes the
//!    controller for slot 3 and stores it there
//!    [OFW-4.93 lv1 a:0x23DA60-0x23DAC4] [OFW-4.93 lv1 a:0x23DBC8-0x23DBD0].
//!    A later init step keeps the device of slot 3 in the controller
//!    [OFW-4.93 lv1 a:0x3AB040-0x3AB068].
//! 2. It calls the controller's open with the caller's partition.
//! 3. It returns that open's r3 zero-extended.
//!
//! The controller's open holds the controller's lock and does these steps:
//! - It calls the device's entry at +16 with the partition
//!   [OFW-4.93 lv1 a:0x23DC54-0x23DC74]. That entry is the open entry of
//!   device 0 [OFW-4.93 lv1 a:0x2280A0-0x2280BC].
//! - When the entry fails, it returns the entry's r3 sign-extended
//!   [OFW-4.93 lv1 a:0x23DC78-0x23DCBC].
//! - Otherwise it asks the device's entry at +48 whether the partition
//!   holds the device open [OFW-4.93 lv1 a:0x23DCC0-0x23DCE0]. That entry
//!   returns 1 when the open flag is set and the stored partition is the
//!   caller's [OFW-4.93 lv1 a:0x228120-0x228144]. A 0 gives -5
//!   [OFW-4.93 lv1 a:0x23DCE4-0x23DCF4].
//! - After a 1, it sets the controller's state word to 2 and returns 0
//!   [OFW-4.93 lv1 a:0x23DD14-0x23DD28]. When the state word was 3, it
//!   first writes the controller's registers
//!   [OFW-4.93 lv1 a:0x23DD2C-0x23DD94].
//!
//! The entry at +48 runs right after the entry at +16 opened the device
//! for the same partition, so it returns 1. No step reads r5, and no hook
//! runs after the open.
//!
//! # Device 2 of bus 1
//!
//! Device 2 takes the path of device 1 with the next slot and the next
//! controller:
//! - the lookup reads it from slot 4 [OFW-4.93 lv1 a:0x235DB0-0x235DB8];
//! - lv1's init fills that slot with the same constructor
//!   [OFW-4.93 lv1 a:0x3A9AA0-0x3A9AC4];
//! - the open function calls the open of the controller in slot 1 of the
//!   array [OFW-4.93 lv1 a:0x237760-0x23777C], which lv1's init makes for
//!   slot 4 [OFW-4.93 lv1 a:0x23DB28-0x23DB54]
//!   [OFW-4.93 lv1 a:0x23DBD4-0x23DBDC].

use super::status::{zero_extended, LV1_BUSY};

/// The handler of hypercall 170 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenDeviceHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `open_device` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 170 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const OPEN_DEVICE_HANDLERS: &[OpenDeviceHandler] = &[OpenDeviceHandler {
    firmware: "4.93",
    handler: 0x305A38,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn open_device_handler(firmware: &str) -> Option<&'static OpenDeviceHandler> {
    OPEN_DEVICE_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest bus id that the handler gives the open function.
pub const OPEN_DEVICE_BUS_MAX: u64 = 4;

/// The r3 of `open_device` for a bus id above [`OPEN_DEVICE_BUS_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x305B78-0x305B84].
pub const OPEN_DEVICE_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 of `open_device` for bus id 0: `LV1_NO_ENTRY` (-6), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x237724-0x237750].
pub const OPEN_DEVICE_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `open_device` when the lookup finds no device:
/// `LV1_NO_ENTRY` (-6), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x237780-0x23778C].
pub const OPEN_DEVICE_NO_DEVICE_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `open_device` for a device that is open: `LV1_BUSY` (-9),
/// zero-extended.
///
/// The device's open entry returns -9 [OFW-4.93 lv1 a:0x2280A4]. The open
/// function clears the high word on each path:
/// - for device 0 of bus 1 [OFW-4.93 lv1 a:0x2376C4];
/// - for device 1 of bus 1, after the controller's open returns the -9
///   [OFW-4.93 lv1 a:0x23DC9C] [OFW-4.93 lv1 a:0x2377EC];
/// - for device 2 of bus 1, on the same terms [OFW-4.93 lv1 a:0x237778].
pub const OPEN_DEVICE_BUSY_R3: u64 = zero_extended(LV1_BUSY);

/// The bus of the devices whose open path CellGov read.
pub const OPEN_DEVICE_READ_BUS: u64 = 1;

/// The device ids, on [`OPEN_DEVICE_READ_BUS`], whose open path CellGov
/// read, ascending.
pub const OPEN_DEVICE_READ_DEVICES: &[u64] = &[0, 1, 2];

/// Whether CellGov read lv1's open path for device `device` of bus `bus`.
pub fn open_device_path_read(bus: u64, device: u64) -> bool {
    bus == OPEN_DEVICE_READ_BUS && OPEN_DEVICE_READ_DEVICES.contains(&device)
}

/// The r3 of the handler's bus checks for the bus id `bus`, or `None`
/// when `bus` passes them:
/// - a bus id above [`OPEN_DEVICE_BUS_MAX`] gets
///   [`OPEN_DEVICE_BUS_RANGE_R3`];
/// - bus id 0 gets [`OPEN_DEVICE_NO_BUS_R3`].
pub const fn open_device_bus_r3(bus: u64) -> Option<u64> {
    if bus > OPEN_DEVICE_BUS_MAX {
        Some(OPEN_DEVICE_BUS_RANGE_R3)
    } else if bus == 0 {
        Some(OPEN_DEVICE_NO_BUS_R3)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "tests/device_bus_open_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/device_bus_open_device_1_tests.rs"]
mod device_1_tests;
