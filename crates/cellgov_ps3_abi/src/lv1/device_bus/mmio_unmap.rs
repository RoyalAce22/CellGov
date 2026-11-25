//! The facts of `unmap_device_mmio_region`: the handler and its checks.
//!
//! # `unmap_device_mmio_region`
//!
//! The handler [OFW-4.93 lv1 a:0x306C10] keeps r3 to r5
//! [OFW-4.93 lv1 a:0x306C24] [OFW-4.93 lv1 a:0x306C3C-0x306C40] and does
//! these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`MMIO_UNMAP_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x306CA0-0x306CA4]
//!    [OFW-4.93 lv1 a:0x306D50-0x306D60].
//! 2. It calls the unmap function [OFW-4.93 lv1 a:0x237168] with the
//!    caller's partition and r3 to r5 [OFW-4.93 lv1 a:0x306CA8-0x306CC4].
//! 3. It returns that function's r3 [OFW-4.93 lv1 a:0x306CC8]
//!    [OFW-4.93 lv1 a:0x306D30]. The handler writes no other slot, and
//!    the vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4
//!    to r10 return 0.
//!
//! The unmap function does these steps. The first failure is the answer:
//! 1. The access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x2371A0-0x2371B0], which
//!    [`network`](crate::lv1::network) describes. Bus 0 gets
//!    [`MMIO_UNMAP_NO_BUS_R3`]. A partition that did not open the device
//!    gets [`MMIO_UNMAP_NOT_OPEN_R3`]. The function zero-extends a failed
//!    status and sets bit (mark + 31) for a mark other than 0
//!    [OFW-4.93 lv1 a:0x2371F0-0x23720C].
//! 2. Device 0 of bus 1 gets [`MMIO_UNMAP_DEVICE_0_R3`]
//!    [OFW-4.93 lv1 a:0x2371B4-0x2371B8] [OFW-4.93 lv1 a:0x237238-0x237250].
//! 3. The device lookup [OFW-4.93 lv1 a:0x2371BC-0x2371D0]. For bus 1 it
//!    is the lookup of the access check, so it finds the device.
//! 4. The device's MMIO unmap function [OFW-4.93 lv1 a:0x228AC4], with
//!    the device, the partition, r5 and a mark
//!    [OFW-4.93 lv1 a:0x2371D4-0x2371E0]. A nonzero r3 gets the treatment
//!    of step 1. On 0 the function returns 0
//!    [OFW-4.93 lv1 a:0x2371E4-0x2371EC].
//!
//! The device's MMIO unmap function holds the device's lock at +16
//! [OFW-4.93 lv1 a:0x228B00-0x228B24] and does these steps:
//! 1. It walks the 8 region slots at +24 of the device. For each slot it
//!    calls the find function with the slot's region, the partition and
//!    r5 [OFW-4.93 lv1 a:0x228B28-0x228B54]. Unlike the walk of
//!    `map_device_mmio_region`, the walk does not skip an empty slot.
//! 2. When no slot gives a mapping, the answer is
//!    [`MMIO_UNMAP_NO_MAPPING_R3`] [OFW-4.93 lv1 a:0x228B58-0x228B60]
//!    [OFW-4.93 lv1 a:0x228B78].
//! 3. For the first mapping found, it frees the address r5 in the
//!    partition [OFW-4.93 lv1 a:0x228BA8-0x228BB8], then unlinks and
//!    destroys the mapping [OFW-4.93 lv1 a:0x228BBC-0x228BC0]. The answer
//!    is the status of the free [OFW-4.93 lv1 a:0x228BC4-0x228BCC]
//!    [OFW-4.93 lv1 a:0x228B78].
//!
//! The find function reads the list of mappings at +72 of the region
//! [OFW-4.93 lv1 a:0x225B14-0x225B1C]. It gives the first mapping whose
//! partition at +8 is the caller's, and whose range holds r5
//! [OFW-4.93 lv1 a:0x225B30-0x225B54]. The range starts at the logical
//! partition address at +48, and its length is at +56. The mapping
//! function of the map stores these fields
//! [OFW-4.93 lv1 a:0x225E88] [OFW-4.93 lv1 a:0x225E94-0x225E98]. The
//! next mapping is at +32 [OFW-4.93 lv1 a:0x225B58-0x225B60].
//!
//! An empty slot holds 0, so the find reads the doubleword at lv1's
//! address 72 as the list. lv1's image holds 0 there
//! [OFW-4.93 lv1 a:0x48-0x4C], so the find of an empty slot gives no
//! mapping.
//!
//! The free [OFW-4.93 lv1 a:0x22996C] gives -6 for an address that its
//! allocator does not hold [OFW-4.93 lv1 a:0x2299C4-0x2299CC]
//! [OFW-4.93 lv1 a:0x229AC0-0x229AF8]. It gives -8 for a region whose
//! class is not the class at r2 - 28080
//! [OFW-4.93 lv1 a:0x229B00-0x229B10]. The address function of the map
//! gives the region that class [OFW-4.93 lv1 a:0x229E3C]
//! [OFW-4.93 lv1 a:0x229E94-0x229EA0]. Before the release, the free calls
//! a method of each user of the region on two lists
//! [OFW-4.93 lv1 a:0x229B50-0x229B84] [OFW-4.93 lv1 a:0x2CFCBC-0x2CFD98].
//! It returns 0 when the release returns 0
//! [OFW-4.93 lv1 a:0x229BF0-0x229C3C].
//!
//! [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX

use crate::lv1::status::{zero_extended, LV1_WRONG_STATE};

/// The handler of hypercall 173 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmioUnmapHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `unmap_device_mmio_region` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 173 in the live 4.93 hypercall
/// table ([`crate::lv1::table::LIVE_TABLE_HANDLERS`]).
pub const MMIO_UNMAP_HANDLERS: &[MmioUnmapHandler] = &[MmioUnmapHandler {
    firmware: "4.93",
    handler: 0x306C10,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn mmio_unmap_handler(firmware: &str) -> Option<&'static MmioUnmapHandler> {
    MMIO_UNMAP_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `unmap_device_mmio_region` for a bus id above
/// [`OPEN_DEVICE_BUS_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x306D50-0x306D5C].
///
/// The high-word bits mark the inputs:
/// - bit 32 marks r3;
/// - bit 33 marks r4;
/// - bit 34 marks r5.
///
/// [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX
pub const MMIO_UNMAP_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 for bus id 0: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x236420-0x236428] [OFW-4.93 lv1 a:0x2371F0-0x23720C].
pub const MMIO_UNMAP_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 when the calling partition did not open the device:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x2371F0-0x2371FC].
pub const MMIO_UNMAP_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 for device 0 of bus 1, which the partition holds open:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x237238-0x237250].
pub const MMIO_UNMAP_DEVICE_0_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 when no mapping of the caller in a region of the device holds
/// the address: `LV1_NO_ENTRY` (-6), with r5 marked
/// [OFW-4.93 lv1 a:0x228B58-0x228B60] [OFW-4.93 lv1 a:0x2371F0-0x23720C].
pub const MMIO_UNMAP_NO_MAPPING_R3: u64 = 0x0000_0004_FFFF_FFFA;
