//! The facts of `close_device`.
//!
//! # `close_device`
//!
//! The handler [OFW-4.93 lv1 a:0x306EEC] does these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`CLOSE_DEVICE_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x306F74-0x306F78]
//!    [OFW-4.93 lv1 a:0x30701C-0x30702C].
//! 2. It calls the close function [OFW-4.93 lv1 a:0x2373B0] with the
//!    caller's partition, r3 and r4 [OFW-4.93 lv1 a:0x306F7C-0x306F94].
//! 3. It returns that function's r3 [OFW-4.93 lv1 a:0x306F98]
//!    [OFW-4.93 lv1 a:0x307000]. It writes no return slot, and the
//!    vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to
//!    r10 return 0.
//!
//! The close function does these steps. The first failure is the answer:
//! 1. It runs the access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x2373E4-0x2373EC], which
//!    [`network`](crate::lv1::network) describes. The function zero-extends a
//!    failed status and sets bit (mark + 31) for a mark other than 0
//!    [OFW-4.93 lv1 a:0x237520-0x237540]. See [`close_device_check_r3`].
//! 2. It looks up the device [OFW-4.93 lv1 a:0x2373F0-0x237408]. The
//!    lookup finds the device that the access check found.
//! 3. Bus 1 takes a path of its own for each of the devices 0, 1 and 2
//!    [OFW-4.93 lv1 a:0x23740C-0x237410] [OFW-4.93 lv1 a:0x237458-0x23745C]
//!    [OFW-4.93 lv1 a:0x2374F8-0x237504]. For each other device, the
//!    function calls the method at +32 of the device's method table, the
//!    close entry, with the partition. It returns that r3 zero-extended
//!    [OFW-4.93 lv1 a:0x237414-0x237434].
//!
//! The access check runs before each step that changes state. So a close
//! of a device that the caller does not hold open changes nothing.
//!
//! ## Device 0 of bus 1
//!
//! The close function holds the lock at +40 of the network device's
//! global object [OFW-4.93 lv1 a:0x237460-0x237498] and does these steps:
//! 1. It runs the close hook [OFW-4.93 lv1 a:0x23E240] with the global
//!    object and the object at +16 [OFW-4.93 lv1 a:0x23749C-0x2374A4]. A
//!    nonzero status of the hook is the answer, zero-extended, and the
//!    device stays open [OFW-4.93 lv1 a:0x2374A8-0x2374B0].
//! 2. Otherwise it calls the device's close entry with the partition. It
//!    returns 0, or the entry's nonzero r3 zero-extended
//!    [OFW-4.93 lv1 a:0x237544-0x237578].
//!
//! The hook does these steps:
//! - An absent object at +16 gives -6 [OFW-4.93 lv1 a:0x23E240-0x23E264].
//! - A state word at +8 other than 2 gives 0 and changes nothing
//!   [OFW-4.93 lv1 a:0x23E268-0x23E278].
//! - A state word of 2 leads to the teardown [OFW-4.93 lv1 a:0x23DFA8] on
//!   each path [OFW-4.93 lv1 a:0x23E294-0x23E2F4].
//!
//! The open of device 0 sets the state word to 2, so the hook tears down
//! the object of a device that the caller holds open.
//!
//! The teardown does these steps on the object:
//! 1. It calls a stop function [OFW-4.93 lv1 a:0x23DFA8-0x23DFD8]. That
//!    function fails with -24 only when a poll of a hardware register stops
//!    after 1000 tries [OFW-4.93 lv1 a:0x243924-0x243968]
//!    [OFW-4.93 lv1 a:0x2439C4]. On that failure the teardown returns the
//!    status, and the state word stays 2 [OFW-4.93 lv1 a:0x23DFDC-0x23E00C].
//! 2. It stops the device's register block [OFW-4.93 lv1 a:0x23E010-0x23E014].
//! 3. When the object's flag byte at +0 is set, it disconnects the event
//!    receive port of the device's interrupt
//!    [OFW-4.93 lv1 a:0x23E018-0x23E020] [OFW-4.93 lv1 a:0x23E128-0x23E138].
//!    The connection is the object of `connect_interrupt_event_receive_port`
//!    [OFW-4.93 lv1 a:0x353CE8-0x353CEC] [OFW-4.93 lv1 a:0x353838-0x35383C].
//!    The disconnect function drops the connection's port
//!    [OFW-4.93 lv1 a:0x245780-0x245848].
//! 4. It destroys the 288-byte object at +24 [OFW-4.93 lv1 a:0x23E024-0x23E044].
//! 5. It stores 0 in the interrupt mask at +32, -1 in the status indicator
//!    at +48 and 0 at +24 [OFW-4.93 lv1 a:0x23E048-0x23E060].
//! 6. When +56 holds the memory region of the status indicator, it
//!    decrements the region's count at +192 and clears +56
//!    [OFW-4.93 lv1 a:0x23E064-0x23E0F4].
//! 7. It stores 3 in the state word and returns 0
//!    [OFW-4.93 lv1 a:0x23E0F8-0x23E108].
//!
//! The capture [`NET_CONTROL_CAPTURE`](crate::lv1::network::NET_CONTROL_CAPTURE)
//! reads 1 in the flag byte of the open device. No input of the call
//! selects the failure of step 1.
//!
//! Device 0's close entry [OFW-4.93 lv1 a:0x2294D4]
//! [OFW-4.93 lv1 a:0x35C898-0x35C89C] [OFW-4.93 lv1 a:0x32D6A8-0x32D6AC]
//! holds the device's lock at +104 and does these steps:
//! 1. A clear open flag at +108, or a partition at +112 other than the
//!    caller's, gives -11 [OFW-4.93 lv1 a:0x22952C-0x229588].
//! 2. It removes the caller's DMA mappings [OFW-4.93 lv1 a:0x226A58]
//!    [OFW-4.93 lv1 a:0x22958C-0x229598]; see below.
//! 3. It frees each DMA region that the caller holds
//!    [OFW-4.93 lv1 a:0x229594-0x229650]; see below.
//! 4. It removes the caller's MMIO mappings [OFW-4.93 lv1 a:0x2292EC]
//!    [OFW-4.93 lv1 a:0x229680-0x229688].
//! 5. It stores -1 at +112, clears the open flag and returns 0
//!    [OFW-4.93 lv1 a:0x22968C-0x2296A0].
//!
//! The unmap step walks each region of the list at +96, lv1's own region
//! too [OFW-4.93 lv1 a:0x226AAC-0x226AC0] [OFW-4.93 lv1 a:0x226B34-0x226B3C].
//! It acts on each mapping whose word at +88 is 0 and whose partition at
//! +8 is the caller's [OFW-4.93 lv1 a:0x226AE0-0x226AF8]. For a partition
//! other than 0, it calls the device's entry at +184 with the bus address
//! of the mapping [OFW-4.93 lv1 a:0x226B78-0x226BA4]
//! [OFW-4.93 lv1 a:0x35C930-0x35C934] [OFW-4.93 lv1 a:0x32D468-0x32D46C].
//! That entry does these steps:
//! - It gets the I/O address of the bus address from the entry at +120,
//!   and returns that entry's failure at once
//!   [OFW-4.93 lv1 a:0x2279F8-0x227A28].
//! - It clears the IOPTEs with the entry at +136
//!   [OFW-4.93 lv1 a:0x227A2C-0x227AA8].
//! - On each path after the IOPTE step, it unlinks the mapping from the
//!   region and destroys it, whether that step failed or not
//!   [OFW-4.93 lv1 a:0x227AAC-0x227AC0] [OFW-4.93 lv1 a:0x227AEC-0x227B30]
//!   [OFW-4.93 lv1 a:0x22584C-0x2258B4].
//!
//! The free step calls the device's free method
//! [OFW-4.93 lv1 a:0x227650] [OFW-4.93 lv1 a:0x35C938-0x35C93C]
//! [OFW-4.93 lv1 a:0x32D408-0x32D40C] for each region whose partition at
//! +8 is the caller's [OFW-4.93 lv1 a:0x2295E8-0x22960C]:
//! - A region that still holds a mapping gets -9 and stays
//!   [OFW-4.93 lv1 a:0x2276B8-0x2276C4]. The close entry skips it
//!   [OFW-4.93 lv1 a:0x229610-0x229614] [OFW-4.93 lv1 a:0x2296A4-0x2296AC].
//! - Otherwise the method releases the region's bus address, unlinks the
//!   region from the list at +96 and destroys it
//!   [OFW-4.93 lv1 a:0x2276C8-0x227768].
//!
//! The entry at +120 gave each mapping of the partition its I/O address
//! when `map_device_dma_region` made the mapping. It used the same bus
//! address and the same device word at +160 then. The mapping function stores
//! 0 at +88, as the capture [`DMA_MAP_CAPTURE`] reads. So the unmap step
//! removes each mapping of the partition, and the free step then frees
//! each region of the partition.
//!
//! ## Devices 1 and 2 of bus 1
//!
//! The close function calls the close of the controller in slot 0 for
//! device 1 and in slot 1 for device 2. It returns that close's r3
//! zero-extended [OFW-4.93 lv1 a:0x23757C-0x23758C]
//! [OFW-4.93 lv1 a:0x237508-0x237518]. The controller's close holds the
//! controller's lock [OFW-4.93 lv1 a:0x23D854-0x23D87C] and does these
//! steps:
//! - When the state word at +240 is not 0 and the device's partition is
//!   the caller's, it clears each flagged slot of the 6 slots at +104. It
//!   then writes 0 to the register at +0xF00 of the controller's register
//!   block [OFW-4.93 lv1 a:0x23D880-0x23D898]
//!   [OFW-4.93 lv1 a:0x23D904-0x23D980].
//! - When the state word is not 0, it stores 3 in it
//!   [OFW-4.93 lv1 a:0x23D89C-0x23D8A0] [OFW-4.93 lv1 a:0x23D8FC-0x23D900].
//! - It calls the device's close entry and returns its r3 sign-extended
//!   [OFW-4.93 lv1 a:0x23D8A4-0x23D8D4].
//!
//! The device's method table is the table of device 0, so the close entry
//! is the close entry of device 0.
//!
//! After a close, the controller's next open writes the controller's
//! registers before it sets the state word to 2
//! [OFW-4.93 lv1 a:0x23DD2C-0x23DD94]. Those registers are in the
//! controller's register block, outside partition memory. That open
//! cannot get -5, for the reason that the section on device 1 of bus 1
//! gives.
//!
//! ## The storage bus
//!
//! For device -1 of bus 4, the close entry is the storage bus object's
//! close method [OFW-4.93 lv1 a:0x259BD8] [OFW-4.93 lv1 a:0x35DD50-0x35DD54]
//! [OFW-4.93 lv1 a:0x335640-0x335644]. Its gate calls the object's method
//! at +48, and then its methods at +232 and +240, which read the state of
//! the storage devices [OFW-4.93 lv1 a:0x253FA0-0x254050]. The method
//! then calls a method of the partition's record through the record's
//! own table [OFW-4.93 lv1 a:0x259CC0-0x259CD4]. CellGov does not model
//! that state.
//!
//! # The 4.93 kernel's calls of `close_device`
//!
//! The kernel sends hypercall 171 at these sites:
//! - with the bus id and the device id at +16 and +8 of a device record
//!   [OFW-4.93 lv2_kernel a:0x80000000000A3570-0x80000000000A357C];
//! - with the bus id and the device id at +144 and +152 of a device record
//!   [OFW-4.93 lv2_kernel a:0x80000000000A90F8-0x80000000000A9104]
//!   [OFW-4.93 lv2_kernel a:0x80000000000AC2C4-0x80000000000AC2D0];
//! - for the storage bus, with r4 = -1
//!   [OFW-4.93 lv2_kernel a:0x80000000000AD594-0x80000000000AD5B4]
//!   [OFW-4.93 lv2_kernel a:0x80000000000AE2F0-0x80000000000AE2FC].
//!
//! [`DMA_MAP_CAPTURE`]: super::DMA_MAP_CAPTURE

use super::open::OPEN_DEVICE_BUS_MAX;
use super::storage_bus::storage_bus_device_absent;
use crate::lv1::status::{zero_extended, LV1_WRONG_STATE};

/// The handler of hypercall 171 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseDeviceHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `close_device` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 171 in the live 4.93 hypercall
/// table ([`crate::lv1::table::LIVE_TABLE_HANDLERS`]).
pub const CLOSE_DEVICE_HANDLERS: &[CloseDeviceHandler] = &[CloseDeviceHandler {
    firmware: "4.93",
    handler: 0x306EEC,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn close_device_handler(firmware: &str) -> Option<&'static CloseDeviceHandler> {
    CLOSE_DEVICE_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `close_device` for a bus id above [`OPEN_DEVICE_BUS_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x30701C-0x30702C].
pub const CLOSE_DEVICE_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 of `close_device` for bus id 0: `LV1_NO_ENTRY` (-6), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x236420-0x236428]
/// [OFW-4.93 lv1 a:0x237520-0x237540].
pub const CLOSE_DEVICE_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `close_device` when the access check finds no device:
/// `LV1_NO_ENTRY` (-6), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x236464-0x236474] [OFW-4.93 lv1 a:0x237520-0x237540].
pub const CLOSE_DEVICE_NO_DEVICE_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `close_device` for a device that the caller does not hold
/// open: `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x237520-0x23752C].
pub const CLOSE_DEVICE_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 of `close_device`'s checks that read no state, for device
/// `device` of bus `bus`, or `None` when the call passes them.
///
/// The checks run in this order:
/// 1. a bus id above [`OPEN_DEVICE_BUS_MAX`] gets
///    [`CLOSE_DEVICE_BUS_RANGE_R3`];
/// 2. bus id 0 gets [`CLOSE_DEVICE_NO_BUS_R3`];
/// 3. a device of bus [`STORAGE_BUS`] that
///    [`storage_bus_device_absent`] names gets
///    [`CLOSE_DEVICE_NO_DEVICE_R3`]. The access check finds no device for
///    it [OFW-4.93 lv1 a:0x2364E8-0x2364F4]
///    [OFW-4.93 lv1 a:0x236554-0x236558].
///
/// [`STORAGE_BUS`]: super::STORAGE_BUS
pub const fn close_device_check_r3(bus: u64, device: u64) -> Option<u64> {
    if bus > OPEN_DEVICE_BUS_MAX {
        Some(CLOSE_DEVICE_BUS_RANGE_R3)
    } else if bus == 0 {
        Some(CLOSE_DEVICE_NO_BUS_R3)
    } else if storage_bus_device_absent(bus, device) {
        Some(CLOSE_DEVICE_NO_DEVICE_R3)
    } else {
        None
    }
}
