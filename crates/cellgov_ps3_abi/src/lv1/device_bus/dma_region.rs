//! The facts of `allocate_device_dma_region`: the checks, the regions that
//! lv1 gave the GameOS kernel's boot, and the slots of devices 1 and 2.
//!
//! # `allocate_device_dma_region`
//!
//! The handler [OFW-4.93 lv1 a:0x306A6C] keeps r3 to r7
//! [OFW-4.93 lv1 a:0x306A80] [OFW-4.93 lv1 a:0x306AA0-0x306AAC] and does
//! these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`DMA_REGION_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x306B0C-0x306B10]
//!    [OFW-4.93 lv1 a:0x306BE8-0x306BF8].
//! 2. It refuses flags above [`DMA_REGION_FLAGS_MAX`] with
//!    [`DMA_REGION_FLAGS_RANGE_R3`] [OFW-4.93 lv1 a:0x306B14-0x306B18]
//!    [OFW-4.93 lv1 a:0x306BFC-0x306C0C].
//! 3. It zeroes the return slot of r4, HSPRG0 - 0x6A38, and calls the
//!    allocate function [OFW-4.93 lv1 a:0x236F88] with the caller's
//!    partition, r3 to r7 and the address of that slot
//!    [OFW-4.93 lv1 a:0x306B1C-0x306B54].
//! 4. It returns that function's r3 [OFW-4.93 lv1 a:0x306B58]
//!    [OFW-4.93 lv1 a:0x306BC0]. The vector's entry zeroes the other
//!    slots [OFW-4.93 lv1 a:0x0C08], so r5 to r10 return 0.
//!
//! The allocate function writes the slot only on success
//! [OFW-4.93 lv1 a:0x23707C-0x237088], so a failure returns r4 = 0. For
//! device 0 of bus 1 it does these steps. The first failure is the
//! answer:
//! 1. The access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x236FE0-0x236FE8], which
//!    [`network`](crate::lv1::network) describes. Bus 0 gets
//!    [`DMA_REGION_NO_BUS_R3`]. A partition that did not open the device
//!    gets [`DMA_REGION_NOT_OPEN_R3`].
//! 2. The device lookup [OFW-4.93 lv1 a:0x236FEC-0x237004]. For bus 1 it
//!    is the lookup of the access check, so it finds device 0.
//! 3. Bus 1 takes a path of its own for devices 1 and 2
//!    [OFW-4.93 lv1 a:0x237008-0x23700C]
//!    [OFW-4.93 lv1 a:0x237108-0x237118]. Device 0 continues.
//! 4. Flags other than 0 get [`DMA_REGION_FLAGS_R3`]
//!    [OFW-4.93 lv1 a:0x237010-0x237024].
//! 5. The device's allocate function [OFW-4.93 lv1 a:0x2267FC], with the
//!    device, the partition, r5, r6 and two outputs
//!    [OFW-4.93 lv1 a:0x237058-0x237070]: the address and a register
//!    mark. On a nonzero r3 the allocate function zero-extends it and
//!    sets bit (mark + 31) for a mark other than 0
//!    [OFW-4.93 lv1 a:0x2370B8-0x2370E4]. On 0 it stores the address in
//!    the slot of r4 and returns 0 [OFW-4.93 lv1 a:0x23707C-0x237088].
//!
//! The device's allocate function holds the device's lock at +88
//! [OFW-4.93 lv1 a:0x226820-0x226844] and does these steps:
//! 1. A size of 0 gets [`DMA_REGION_SIZE_R3`]
//!    [OFW-4.93 lv1 a:0x226848-0x22684C] [OFW-4.93 lv1 a:0x226928-0x226930].
//! 2. A page shift other than those of [`DMA_REGION_PAGE_SHIFTS`] gets
//!    [`DMA_REGION_PAGE_SHIFT_R3`] [OFW-4.93 lv1 a:0x226850-0x226878]
//!    [OFW-4.93 lv1 a:0x226964-0x22696C].
//! 3. A size that is not a multiple of the page gets
//!    [`DMA_REGION_SIZE_R3`] [OFW-4.93 lv1 a:0x226880-0x22688C].
//! 4. It clears the mark and calls the device's entry at +152 with the
//!    device, the size, the page shift and the partition
//!    [OFW-4.93 lv1 a:0x226890-0x2268B0]. A nonzero r3 of the entry is
//!    the answer [OFW-4.93 lv1 a:0x2268B4-0x2268D0].
//! 5. Otherwise it gives the caller +48 of the region that the entry
//!    made [OFW-4.93 lv1 a:0x2268EC-0x2268F4].
//!
//! Device 0's entry at +152 [OFW-4.93 lv1 a:0x35C910-0x35C914]
//! [OFW-4.93 lv1 a:0x32D300-0x32D304] does these steps:
//! 1. It calls the device's entry at +88 [OFW-4.93 lv1 a:0x226BF0-0x226C1C],
//!    which picks the bus address; see below. A nonzero r3 is the answer,
//!    sign-extended [OFW-4.93 lv1 a:0x226C20-0x226C58].
//! 2. It allocates an 80-byte region [OFW-4.93 lv1 a:0x226C5C-0x226C70].
//!    It fills the region [OFW-4.93 lv1 a:0x225CAC-0x225CDC] with the
//!    partition at +8, the address at +16 and +48, the size at +56 and
//!    the page shift at +64.
//! 3. It links the region into the device's list at +96 and returns 0
//!    [OFW-4.93 lv1 a:0x226C8C-0x226D0C].
//!
//! When the allocation of the region fails, the entry returns
//! `LV1_RESOURCE_SHORTAGE` (-2) [OFW-4.93 lv1 a:0x226D88-0x226D94]. No
//! input of the call selects that case.
//!
//! Device 0's entry at +88 [OFW-4.93 lv1 a:0x35C8D0-0x35C8D4]
//! [OFW-4.93 lv1 a:0x32E0E0-0x32E0E4] does these steps:
//! 1. When +120 of the device holds -1, it returns -17
//!    [OFW-4.93 lv1 a:0x22D990-0x22D9B0]. lv1's init fills +120
//!    [OFW-4.93 lv1 a:0x22DDF8-0x22DE08], and the capture
//!    [`DMA_REGION_CAPTURE`] reads 2 there.
//! 2. It calls the pool function [OFW-4.93 lv1 a:0x22D3A4] with the
//!    device's backend at +136 and index at +128
//!    [OFW-4.93 lv1 a:0x22D9B4-0x22D9BC]. lv1's init sets them to the
//!    system bus object and 0 [OFW-4.93 lv1 a:0x3A9924-0x3A9938]
//!    [OFW-4.93 lv1 a:0x3A9768-0x3A9780] [OFW-4.93 lv1 a:0x22DDD8-0x22DDDC].
//!    So the pool is slot 0 of that object, the device itself.
//! 3. The pool function rounds the size up to a power of two; see
//!    [`dma_region_rounded_size`]. A rounded size above
//!    [`DMA_REGION_SIZE_MAX`] gets -17 [OFW-4.93 lv1 a:0x22D3FC-0x22D42C]
//!    [OFW-4.93 lv1 a:0x22D4B8-0x22D4D0] [OFW-4.93 lv1 a:0x22D56C-0x22D580].
//!    The allocate function zero-extends it with no mark, so the answer
//!    is [`DMA_REGION_TOO_LARGE_R3`].
//! 4. It searches the pool for a free range of that size
//!    [OFW-4.93 lv1 a:0x22D434-0x22D440] [OFW-4.93 lv1 a:0x22CEF8]. The
//!    search walks lists that lv1's own allocations change, under a
//!    global lock [OFW-4.93 lv1 a:0x22AC9C-0x22AD44].
//! 5. For index 0 it subtracts the sum of the pool's words at +168 and
//!    +176 from the found address, and adds 0x80000000
//!    [OFW-4.93 lv1 a:0x22D584-0x22D598] [OFW-4.93 lv1 a:0x22D6B8-0x22D6F0].
//!
//! The bus address therefore depends on the pool's state, which
//! CellGov does not model. [`GAME_OS_DMA_REGIONS`] gives the address
//! that the console's lv1 gave the 4.93 kernel's call.
//!
//! # `allocate_device_dma_region` for devices 1 and 2 of bus 1
//!
//! The allocate function sends the devices of [`DMA_SLOT_DEVICES`] to a
//! path of their own [OFW-4.93 lv1 a:0x237008-0x23700C]
//! [OFW-4.93 lv1 a:0x237108-0x23711C]. The path has no flags check. It
//! does these steps:
//! 1. It takes the device's slot object, the controller object that
//!    `open_device` uses, from a table: entry 0 for device 1 and entry 1
//!    for device 2 [OFW-4.93 lv1 a:0x23710C-0x237120]
//!    [OFW-4.93 lv1 a:0x23D0F8-0x23D104]. The TOC slot at r2 - 25520
//!    holds the table's address, 0x3682F8 [OFW-4.93 lv1 a:0x353C88-0x353C8C].
//! 2. It calls the slot function [OFW-4.93 lv1 a:0x23D4C4] with the slot
//!    object, the partition, r5, r6, r7 and two outputs
//!    [OFW-4.93 lv1 a:0x237124-0x23713C]. On 0 it stores the address as
//!    device 0's path does [OFW-4.93 lv1 a:0x237140-0x237144]. A nonzero
//!    r3 gets device 0's mark handling [OFW-4.93 lv1 a:0x237148-0x237164].
//!
//! The slot function holds the lock at +0 of the slot object
//! [OFW-4.93 lv1 a:0x23D508-0x23D520] and does these steps:
//! 1. It calls the device's allocate function [OFW-4.93 lv1 a:0x2267FC]
//!    for the device at +24 of the slot object
//!    [OFW-4.93 lv1 a:0x23D524-0x23D540]. A nonzero r3 is the answer, and
//!    no slot changes [OFW-4.93 lv1 a:0x23D544-0x23D548].
//! 2. It searches the six slots at +104, in order, for a free one
//!    [OFW-4.93 lv1 a:0x23D54C-0x23D578]. When all six are taken, it gives
//!    the region back and returns -2 [OFW-4.93 lv1 a:0x23D57C-0x23D590].
//! 3. It marks the slot and stores the partition and the low word of the
//!    address in it [OFW-4.93 lv1 a:0x23D5D0-0x23D5FC].
//! 4. It writes the flags, the size and the low word of the address to
//!    the slot object's register block
//!    [OFW-4.93 lv1 a:0x23D600] [OFW-4.93 lv1 a:0x23D3A8-0x23D4C0]. That
//!    function returns 0, so the slot function returns 0
//!    [OFW-4.93 lv1 a:0x23D604-0x23D608].
//!
//! The device at +24 of device 1's slot object is the device that the
//! access check finds. The lookup gives device 1 the code 3
//! [OFW-4.93 lv1 a:0x235DA4-0x235DAC], which selects +56 of the system bus
//! object [OFW-4.93 lv1 a:0x22B838-0x22B844]. The capture
//! [`DMA_SLOTS_CAPTURE`] reads these facts:
//! - The device's method table is device 0's, so the entries that the
//!   access check calls and the entries at +152 and +88 that reach the
//!   pool function are device 0's. The size and page shift checks run in
//!   the device's allocate function before any entry of the table.
//! - The device's +120 holds 5, so the device accepts a region.
//! - The pool index at +128 is 3, so the pool differs from device 0's.
//!
//! [`GAME_OS_DMA_REGIONS`] gives the addresses that the console's lv1 gave
//! the 4.93 kernel's five calls for device 1, in the order of the calls.
//!
//! ## The order of the regions
//!
//! The pool search walks lists that lv1's own allocations change
//! [OFW-4.93 lv1 a:0x22D434-0x22D440] [OFW-4.93 lv1 a:0x22CEF8]. So the
//! address of a region depends on the regions that lv1 gave before it,
//! and a captured address holds only for a call that comes in the
//! captured order. The capture [`DMA_SLOTS_CAPTURE`] gives that order:
//! - The device's list holds the five regions with region 1 at its head,
//!   and each region links to the region before it. The link step adds
//!   each new region at the tail [OFW-4.93 lv1 a:0x226CA4-0x226CDC]
//!   [OFW-4.93 lv1 a:0x226D6C-0x226D78], because the method of the region
//!   class 0x35C560 that it calls for each region returns 0
//!   [OFW-4.93 lv1 a:0x35C570-0x35C574] [OFW-4.93 lv1 a:0x32CFA0-0x32CFA4]
//!   [OFW-4.93 lv1 a:0x2258F8-0x2258FC].
//!   So the list is in the order of the calls.
//! - Slots 0 to 4 hold the regions in the order of the list, and slot 5
//!   is free. The slot function takes the first free slot, so the slots
//!   are in the order of the calls.
//!
//! A model that matched a call to a row by its size and page shift alone
//! would give a row's address to a call that comes out of that order.
//!
//! When all [`DMA_SLOT_COUNT`] slots are taken, the slot function gives
//! the region back and returns `LV1_RESOURCE_SHORTAGE` (-2)
//! [OFW-4.93 lv1 a:0x23D57C-0x23D590]. It searches the slots only after
//! the pool gave a region [OFW-4.93 lv1 a:0x23D544-0x23D548], so the pool's
//! own failure comes first.
//!
//! # The 4.93 kernel's call of `allocate_device_dma_region`
//!
//! The kernel's wrapper sends the call with these inputs
//! [OFW-4.93 lv2_kernel a:0x8000000000095848-0x800000000009585C]:
//! - r3, r4 and r5: its caller's bus id, device id and size;
//! - r6: the base-2 logarithm of the page size, 12, 16, 20 or 24
//!   [OFW-4.93 lv2_kernel a:0x8000000000095794-0x80000000000957FC];
//! - r7: its caller's flags [OFW-4.93 lv2_kernel a:0x8000000000095830]
//!   [OFW-4.93 lv2_kernel a:0x8000000000095854].
//!
//! The network device's caller takes the bus id and the device id from
//! +144 and +152 of its device record and sends flags 0
//! [OFW-4.93 lv2_kernel a:0x80000000000AA488-0x80000000000AA4A0]. The
//! call for device 1 has flags 1, so another caller sends it. The README
//! of [`DMA_SLOTS_CAPTURE`] gives its inputs.
//!
//! When r3 is 0, the wrapper stores r4 as the region's address
//! [OFW-4.93 lv2_kernel a:0x8000000000095860-0x8000000000095880].
//!
//! [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX

use crate::lv1::status::{zero_extended, LV1_ILLEGAL_PARAMETER_VALUE, LV1_WRONG_STATE};

/// The handler of hypercall 174 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaRegionHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `allocate_device_dma_region` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 174 in the live 4.93 hypercall
/// table ([`crate::lv1::table::LIVE_TABLE_HANDLERS`]).
pub const DMA_REGION_HANDLERS: &[DmaRegionHandler] = &[DmaRegionHandler {
    firmware: "4.93",
    handler: 0x306A6C,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn dma_region_handler(firmware: &str) -> Option<&'static DmaRegionHandler> {
    DMA_REGION_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `allocate_device_dma_region` for a bus id above
/// [`OPEN_DEVICE_BUS_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x306BE8-0x306BF4].
///
/// Bit 32 marks r3, bit 33 r4, bit 34 r5, bit 35 r6 and bit 36 r7.
///
/// [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX
pub const DMA_REGION_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The largest flags value that the handler passes, unsigned
/// [OFW-4.93 lv1 a:0x306B14-0x306B18].
pub const DMA_REGION_FLAGS_MAX: u64 = 3;

/// The r3 for flags above [`DMA_REGION_FLAGS_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r7 marked
/// [OFW-4.93 lv1 a:0x306BFC-0x306C08].
pub const DMA_REGION_FLAGS_RANGE_R3: u64 = 0x0000_0010_FFFF_FFEF;

/// The r3 for bus id 0: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x236420-0x236428] [OFW-4.93 lv1 a:0x2370B8-0x2370E4].
pub const DMA_REGION_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 when the calling partition did not open the device:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x2370B8-0x2370C4].
pub const DMA_REGION_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 for device 0 of bus 1 with flags other than 0:
/// `LV1_UNSUPPORTED_PARAMETER_VALUE` (-15), with r7 marked
/// [OFW-4.93 lv1 a:0x237018-0x237024].
pub const DMA_REGION_FLAGS_R3: u64 = 0x0000_0010_FFFF_FFF1;

/// The r3 for a size of 0 or a size that is not a multiple of the page:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x226928-0x226930] [OFW-4.93 lv1 a:0x2370B8-0x2370E4].
pub const DMA_REGION_SIZE_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 for a page shift that is not in [`DMA_REGION_PAGE_SHIFTS`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r6 marked
/// [OFW-4.93 lv1 a:0x226964-0x22696C] [OFW-4.93 lv1 a:0x2370B8-0x2370E4].
pub const DMA_REGION_PAGE_SHIFT_R3: u64 = 0x0000_0008_FFFF_FFEF;

/// The page shifts that the device's allocate function accepts: pages of
/// 4 KB, 64 KB, 1 MB and 16 MB [OFW-4.93 lv1 a:0x226850-0x22687C].
pub const DMA_REGION_PAGE_SHIFTS: [u64; 4] = [12, 16, 20, 24];

/// The largest rounded size that the pool function accepts
/// [OFW-4.93 lv1 a:0x22D420-0x22D42C] [OFW-4.93 lv1 a:0x22D4C0-0x22D4CC].
pub const DMA_REGION_SIZE_MAX: u64 = 1 << 32;

/// The r3 for a rounded size above [`DMA_REGION_SIZE_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), zero-extended
/// [OFW-4.93 lv1 a:0x22D4D0] [OFW-4.93 lv1 a:0x2370B8-0x2370C4].
pub const DMA_REGION_TOO_LARGE_R3: u64 = zero_extended(LV1_ILLEGAL_PARAMETER_VALUE);

/// The size that the pool function searches for, from the nonzero size
/// `size`.
///
/// A power of two stays as it is [OFW-4.93 lv1 a:0x22D404-0x22D41C].
/// Another size becomes twice its highest set bit
/// [OFW-4.93 lv1 a:0x22D4B8-0x22D4BC] [OFW-4.93 lv1 a:0x22D56C-0x22D570].
/// lv1 doubles at 64 bits, so a size above 2^63 that is not a power of
/// two becomes 0.
pub const fn dma_region_rounded_size(size: u64) -> u64 {
    if size == 0 {
        return 0;
    }
    let top = 1u64 << (63 - size.leading_zeros());
    if top == size {
        size
    } else {
        top.wrapping_shl(1)
    }
}

/// The r3 of the size and page shift checks for `size` and `page_shift`,
/// or `None` when they pass.
///
/// The checks run in this order:
/// 1. a size of 0 gets [`DMA_REGION_SIZE_R3`];
/// 2. a page shift that is not in [`DMA_REGION_PAGE_SHIFTS`] gets
///    [`DMA_REGION_PAGE_SHIFT_R3`];
/// 3. a size that is not a multiple of the page gets
///    [`DMA_REGION_SIZE_R3`];
/// 4. a rounded size above [`DMA_REGION_SIZE_MAX`] gets
///    [`DMA_REGION_TOO_LARGE_R3`].
///
/// lv1 compares each value at 64 bits.
pub const fn dma_region_shape_r3(size: u64, page_shift: u64) -> Option<u64> {
    if size == 0 {
        return Some(DMA_REGION_SIZE_R3);
    }
    let shifts = DMA_REGION_PAGE_SHIFTS;
    if page_shift != shifts[0]
        && page_shift != shifts[1]
        && page_shift != shifts[2]
        && page_shift != shifts[3]
    {
        return Some(DMA_REGION_PAGE_SHIFT_R3);
    }
    if !size.is_multiple_of(1u64 << page_shift) {
        return Some(DMA_REGION_SIZE_R3);
    }
    if dma_region_rounded_size(size) > DMA_REGION_SIZE_MAX {
        return Some(DMA_REGION_TOO_LARGE_R3);
    }
    None
}

/// The DMA region that the console's lv1 gave one call of the GameOS
/// kernel's boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsDmaRegion {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the call.
    pub bus: u64,
    /// The device id of the call.
    pub device: u64,
    /// The size of the call, in bytes.
    pub size: u64,
    /// The page shift of the call.
    pub page_shift: u64,
    /// The bus address that lv1 returned in r4.
    pub address: u64,
}

/// Each region of a kernel's boot calls, by firmware.
///
/// The rows of one device are in the order of the device's calls. The
/// rows are these:
/// - The network device's 1 MB region of 1 MB pages. The capture
///   [`DMA_REGION_CAPTURE`] finds it in the device's list, with the game
///   OS partition at +8 and the address at +48.
/// - Device 1's five regions. The capture [`DMA_SLOTS_CAPTURE`] finds
///   them in the device's list and in the device's slots 0 to 4, in this
///   order.
pub const GAME_OS_DMA_REGIONS: &[GameOsDmaRegion] = &[
    GameOsDmaRegion {
        firmware: "4.93",
        bus: 1,
        device: 0,
        size: 0x10_0000,
        page_shift: 20,
        address: 0xC000_0000,
    },
    GameOsDmaRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        size: 0x4_0000,
        page_shift: 16,
        address: 0xC000_0000,
    },
    GameOsDmaRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        size: 0x8_0000,
        page_shift: 16,
        address: 0xC008_0000,
    },
    GameOsDmaRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        size: 0x80_0000,
        page_shift: 16,
        address: 0xC080_0000,
    },
    GameOsDmaRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        size: 0x80_0000,
        page_shift: 20,
        address: 0xD000_0000,
    },
    GameOsDmaRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        size: 0x1_0000,
        page_shift: 12,
        address: 0xE000_0000,
    },
];

/// The devices of bus 1 whose regions lv1 also keeps in a slot object:
/// devices 1 and 2 [OFW-4.93 lv1 a:0x237108-0x23711C].
pub const DMA_SLOT_DEVICES: [u64; 2] = [1, 2];

/// The number of slots in the slot object of device 1 or 2
/// [OFW-4.93 lv1 a:0x23D54C-0x23D578].
pub const DMA_SLOT_COUNT: usize = 6;

/// The regions that lv1 gave the kernel's boot calls for device `device`
/// of bus `bus`, in the order of the calls.
pub fn game_os_dma_regions(
    firmware: &str,
    bus: u64,
    device: u64,
) -> impl Iterator<Item = GameOsDmaRegion> + '_ {
    GAME_OS_DMA_REGIONS.iter().copied().filter(move |region| {
        region.firmware == firmware && region.bus == bus && region.device == device
    })
}

/// The console capture of the DMA regions that lv1 holds for the network
/// device.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const DMA_REGION_CAPTURE: &str = "tests/micro/lv1_device_dma_region";

/// The console capture of device 1's slot object, the device and its DMA
/// regions.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const DMA_SLOTS_CAPTURE: &str = "tests/micro/lv1_device_dma_slots";
