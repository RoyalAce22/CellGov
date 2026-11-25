//! The facts of `map_device_mmio_region`: the handler and its checks.
//!
//! # `map_device_mmio_region`
//!
//! The handler [OFW-4.93 lv1 a:0x306D64] keeps r3 to r7
//! [OFW-4.93 lv1 a:0x306D78] [OFW-4.93 lv1 a:0x306D98-0x306DA4] and does
//! these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`MMIO_MAP_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x306E04-0x306E08]
//!    [OFW-4.93 lv1 a:0x306ED8-0x306EE8].
//! 2. It zeroes the return slot of r4, HSPRG0 - 0x6A38, and calls the map
//!    function [OFW-4.93 lv1 a:0x237254] with the caller's partition, r3
//!    to r7 and the address of that slot
//!    [OFW-4.93 lv1 a:0x306E0C-0x306E44].
//! 3. It returns that function's r3 [OFW-4.93 lv1 a:0x306E48]
//!    [OFW-4.93 lv1 a:0x306EB0]. The vector's entry zeroes the other
//!    slots [OFW-4.93 lv1 a:0x0C08], so r5 to r10 return 0.
//!
//! The map function writes the slot only on success
//! [OFW-4.93 lv1 a:0x237300-0x237308], so a failure returns r4 = 0. It
//! does these steps. The first failure is the answer:
//! 1. The access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x2372AC-0x2372B4], which
//!    [`network`](crate::lv1::network) describes. Bus 0 gets
//!    [`MMIO_MAP_NO_BUS_R3`]. A partition that did not open the device
//!    gets [`MMIO_MAP_NOT_OPEN_R3`]. The function zero-extends a failed
//!    status and sets bit (mark + 31) for a mark other than 0
//!    [OFW-4.93 lv1 a:0x237340-0x237390].
//! 2. Device 0 of bus 1 gets [`MMIO_MAP_DEVICE_0_R3`]
//!    [OFW-4.93 lv1 a:0x2372B8-0x2372BC] [OFW-4.93 lv1 a:0x237394-0x2373AC].
//! 3. The device lookup [OFW-4.93 lv1 a:0x2372C0-0x2372D4]. For bus 1 it
//!    is the lookup of the access check, so it finds the device.
//! 4. The device's MMIO map function [OFW-4.93 lv1 a:0x228BEC], with the
//!    device, the partition, r5, r6, r7, two outputs and a mark
//!    [OFW-4.93 lv1 a:0x2372D8-0x2372F4]. A nonzero r3 gets the treatment
//!    of step 1 [OFW-4.93 lv1 a:0x2372F8-0x2372FC]. On 0 the function
//!    stores the logical partition address in the slot of r4 and returns
//!    0 [OFW-4.93 lv1 a:0x237300-0x237308].
//!
//! The device's MMIO map function holds the device's lock at +16
//! [OFW-4.93 lv1 a:0x228C48-0x228C6C] and does these steps:
//! 1. It walks the 8 region slots at +24 of the device for the region
//!    whose bus address at +48 is r5 [OFW-4.93 lv1 a:0x228C70-0x228CA0].
//!    When no region has that address, the answer is
//!    [`MMIO_MAP_NO_REGION_R3`] [OFW-4.93 lv1 a:0x228CA4-0x228CAC].
//! 2. A size of 0 or above the region's size at +56 gets
//!    [`MMIO_MAP_SIZE_R3`] [OFW-4.93 lv1 a:0x228D04-0x228D14]
//!    [OFW-4.93 lv1 a:0x228DF4-0x228E00].
//! 3. A page shift other than those of [`MMIO_MAP_PAGE_SHIFTS`] gets
//!    [`MMIO_MAP_PAGE_SHIFT_R3`] [OFW-4.93 lv1 a:0x228D18-0x228D34]
//!    [OFW-4.93 lv1 a:0x228DE4-0x228DF0].
//! 4. It calls the device's entry at +128 with the bus address
//!    [OFW-4.93 lv1 a:0x228D38-0x228D5C] [OFW-4.93 lv1 a:0x35C8F8-0x35C8FC]
//!    [OFW-4.93 lv1 a:0x32E0F8-0x32E0FC] [OFW-4.93 lv1 a:0x22D9C0-0x22D9D0].
//!    That entry gives the real address of the bus address, and returns
//!    0 on each path [OFW-4.93 lv1 a:0x22BE1C-0x22BE68].
//! 5. It calls the address function [OFW-4.93 lv1 a:0x229CA8] with the
//!    partition, the real address, the page shift, the size, an output
//!    and the mark [OFW-4.93 lv1 a:0x228D68-0x228D80]. A nonzero r3 of
//!    that function is the answer [OFW-4.93 lv1 a:0x228D84-0x228D8C].
//! 6. It makes the mapping with the mapping function of the DMA path
//!    [OFW-4.93 lv1 a:0x225E18], with the region, the partition, the
//!    region's bus address, the size, the logical partition address and 1
//!    in r9 and r10 [OFW-4.93 lv1 a:0x228D90-0x228DB8].
//! 7. It clears the mark, stores the logical partition address at +80 of
//!    the mapping, gives that address to the caller and returns 0
//!    [OFW-4.93 lv1 a:0x228DC8-0x228DE0] [OFW-4.93 lv1 a:0x228CB0-0x228CC4].
//!
//! The address function does these steps:
//! 1. A size that is not a multiple of 2^(page shift) gets -17
//!    [OFW-4.93 lv1 a:0x229E08-0x229E1C] [OFW-4.93 lv1 a:0x22A01C-0x22A020],
//!    which the call answers with
//!    [`MMIO_MAP_SIZE_R3`] [OFW-4.93 lv1 a:0x228D84-0x228D8C].
//! 2. It takes the logical partition address from the partition's region
//!    allocator [OFW-4.93 lv1 a:0x229E78-0x229E80], with the allocator
//!    object of the partition's entry [OFW-4.93 lv1 a:0x229E50-0x229E74].
//!    It gives the address only on success
//!    [OFW-4.93 lv1 a:0x229F84-0x229F98].
//!
//! The allocator takes the value of its counter and increments the
//! counter [OFW-4.93 lv1 a:0x2CED84-0x2CED94]. It makes the address from
//! the size and that value in the encoding of [`crate::lv1::memory`]
//! [OFW-4.93 lv1 a:0x2CED40-0x2CED48] [OFW-4.93 lv1 a:0x2CED6C]
//! [OFW-4.93 lv1 a:0x2CED98-0x2CEDA8]. The rows of
//! [`GAME_OS_MMIO_MAPPINGS`] hold the next four counter values after the
//! kernel's earlier regions, so the counter is the partition's region
//! counter.
//!
//! With r10 not 0, the mapping function skips the range check
//! [OFW-4.93 lv1 a:0x225E18] [OFW-4.93 lv1 a:0x225E5C]. That check
//! refuses a range outside the region
//! [OFW-4.93 lv1 a:0x225F7C-0x225F84] [OFW-4.93 lv1 a:0x225FE4-0x225FF4]
//! and a range that overlaps a mapping of the region
//! [OFW-4.93 lv1 a:0x225FF8-0x226048]. So a mapping of lv1's own in the
//! region does not stop the call. The function keeps r9 in the byte at
//! +64 of the mapping [OFW-4.93 lv1 a:0x225E9C]. When its
//! 96-byte allocation fails [OFW-4.93 lv1 a:0x225E60-0x225E74], the map
//! function frees the logical partition address and returns
//! `LV1_RESOURCE_SHORTAGE` (-2) [OFW-4.93 lv1 a:0x228DC4]
//! [OFW-4.93 lv1 a:0x228E20-0x228E34]. No input of the call selects that
//! case.
//!
//! lv1's init gives devices 1 and 2 of bus 1 three regions each; see
//! [`MMIO_REGIONS`]. The region function stores region i in the slot at
//! +24 + 8 x i. It makes each region an 80-byte object with the page
//! shift 20 [OFW-4.93 lv1 a:0x227F4C-0x227FC8]. The captures
//! [`MMIO_MAP_CAPTURE`] and [`MMIO_DEVICE_2_CAPTURE`] read these regions
//! in the devices' slots.
//!
//! The close entry that devices 0, 1 and 2 of bus 1 share removes the
//! caller's MMIO mappings [OFW-4.93 lv1 a:0x2292EC]
//! [OFW-4.93 lv1 a:0x229680-0x229688]. For each region of the device, it
//! frees the logical partition address of each mapping of the caller,
//! and unlinks and destroys the mapping [OFW-4.93 lv1 a:0x229360-0x229448].
//!
//! [`GAME_OS_MMIO_MAPPINGS`]: super::GAME_OS_MMIO_MAPPINGS
//! [`MMIO_DEVICE_2_CAPTURE`]: super::MMIO_DEVICE_2_CAPTURE
//! [`MMIO_MAP_CAPTURE`]: super::MMIO_MAP_CAPTURE
//! [`MMIO_REGIONS`]: super::MMIO_REGIONS
//! [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX

use crate::lv1::status::{zero_extended, LV1_WRONG_STATE};

/// The handler of hypercall 172 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmioMapHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `map_device_mmio_region` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 172 in the live 4.93 hypercall
/// table ([`crate::lv1::table::LIVE_TABLE_HANDLERS`]).
pub const MMIO_MAP_HANDLERS: &[MmioMapHandler] = &[MmioMapHandler {
    firmware: "4.93",
    handler: 0x306D64,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn mmio_map_handler(firmware: &str) -> Option<&'static MmioMapHandler> {
    MMIO_MAP_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `map_device_mmio_region` for a bus id above
/// [`OPEN_DEVICE_BUS_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x306ED8-0x306EE4].
///
/// The high-word bits mark the inputs:
/// - bit 32 marks r3;
/// - bit 33 marks r4;
/// - bit 34 marks r5;
/// - bit 35 marks r6;
/// - bit 36 marks r7.
///
/// [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX
pub const MMIO_MAP_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 for bus id 0: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x236420-0x236428] [OFW-4.93 lv1 a:0x237340-0x237390].
pub const MMIO_MAP_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 when the calling partition did not open the device:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x237340-0x23734C].
pub const MMIO_MAP_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 for device 0 of bus 1, which the partition holds open:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x237394-0x2373AC].
pub const MMIO_MAP_DEVICE_0_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 when no region of the device has the bus address:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x228CA4-0x228CAC] [OFW-4.93 lv1 a:0x237340-0x237390].
pub const MMIO_MAP_NO_REGION_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 for a size of 0 or above the region's size:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r6 marked
/// [OFW-4.93 lv1 a:0x228DF4-0x228E00] [OFW-4.93 lv1 a:0x237340-0x237390].
///
/// A size that is not a multiple of the page size gets the same r3. The
/// address function returns -17 and stores the mark of r6 in the mark
/// slot of the MMIO map function
/// [OFW-4.93 lv1 a:0x22A01C-0x22A028] [OFW-4.93 lv1 a:0x228C44]
/// [OFW-4.93 lv1 a:0x228D7C].
pub const MMIO_MAP_SIZE_R3: u64 = 0x0000_0008_FFFF_FFEF;

/// The r3 for a page shift outside [`MMIO_MAP_PAGE_SHIFTS`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r7 marked
/// [OFW-4.93 lv1 a:0x228DE4-0x228DF0] [OFW-4.93 lv1 a:0x237340-0x237390].
pub const MMIO_MAP_PAGE_SHIFT_R3: u64 = 0x0000_0010_FFFF_FFEF;

/// The page shifts that the device's MMIO map function accepts
/// [OFW-4.93 lv1 a:0x228D18-0x228D34].
pub const MMIO_MAP_PAGE_SHIFTS: [u64; 4] = [12, 16, 20, 24];
