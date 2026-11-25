//! The facts of `unmap_device_dma_region`: the handler, its checks and
//! the 4.93 kernel's calls.
//!
//! # `unmap_device_dma_region`
//!
//! The handler [OFW-4.93 lv1 a:0x30660C] keeps r3 to r6
//! [OFW-4.93 lv1 a:0x306620] [OFW-4.93 lv1 a:0x30663C-0x306644] and does
//! these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`DMA_UNMAP_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x3066A4-0x3066A8]
//!    [OFW-4.93 lv1 a:0x30675C-0x30676C].
//! 2. It calls the unmap function [OFW-4.93 lv1 a:0x236C1C] with the
//!    caller's partition and r3 to r6 [OFW-4.93 lv1 a:0x3066AC-0x3066CC].
//! 3. It returns that function's r3 [OFW-4.93 lv1 a:0x3066D0]
//!    [OFW-4.93 lv1 a:0x306738]. The handler writes no other slot, and
//!    the vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4
//!    to r10 return 0.
//!
//! The unmap function does these steps. The first failure is the answer:
//! 1. The access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x236C64], which
//!    [`network`](crate::lv1::network) describes. Bus 0 gets
//!    [`DMA_UNMAP_NO_BUS_R3`]. A partition that did not open the device
//!    gets [`DMA_UNMAP_NOT_OPEN_R3`]. The function zero-extends a failed
//!    status and sets bit (mark + 31) for a mark other than 0
//!    [OFW-4.93 lv1 a:0x236C78-0x236C7C] [OFW-4.93 lv1 a:0x236CFC-0x236D18].
//! 2. The device lookup [OFW-4.93 lv1 a:0x236C80]. A miss gives -6
//!    sign-extended, with no mark [OFW-4.93 lv1 a:0x236C98-0x236C9C]
//!    [OFW-4.93 lv1 a:0x236CD4]. For devices 0, 1 and 2
//!    of bus 1, the lookup finds the device that `map_device_dma_region`
//!    finds.
//! 3. The device's unmap function [OFW-4.93 lv1 a:0x226478], with the
//!    device, the partition, r5, r6 and a mark
//!    [OFW-4.93 lv1 a:0x236C84-0x236CA0]. On 0 the function returns 0
//!    [OFW-4.93 lv1 a:0x236CA4-0x236CAC]. A nonzero r3 gets the treatment
//!    of step 1 [OFW-4.93 lv1 a:0x236CB0-0x236CCC].
//!
//! The device's unmap function holds the device's lock at +88
//! [OFW-4.93 lv1 a:0x2264BC-0x2264E0] and does these steps:
//! 1. It finds the mapping that holds the bus address with the device's
//!    entry at +208 [OFW-4.93 lv1 a:0x2264E4-0x22650C]
//!    [OFW-4.93 lv1 a:0x35C948-0x35C94C] [OFW-4.93 lv1 a:0x32D348-0x32D34C].
//!    When no mapping holds it, the answer is [`DMA_UNMAP_NO_MAPPING_R3`]
//!    [OFW-4.93 lv1 a:0x226680-0x22668C].
//! 2. A mapping whose doubleword at +88 is not 0 gets `LV1_BUSY` (-9),
//!    zero-extended [OFW-4.93 lv1 a:0x226510-0x226518]
//!    [OFW-4.93 lv1 a:0x226654-0x226660].
//! 3. A bus address other than the mapping's first byte gets
//!    [`DMA_UNMAP_ADDRESS_R3`] [OFW-4.93 lv1 a:0x22651C-0x226530]
//!    [OFW-4.93 lv1 a:0x226644-0x226650].
//! 4. A length other than the mapping's length gets
//!    [`DMA_UNMAP_LENGTH_R3`] [OFW-4.93 lv1 a:0x226534-0x22653C]
//!    [OFW-4.93 lv1 a:0x226634-0x226640].
//! 5. It finds the region of the bus address with the entry at +200, and
//!    checks that the bus address and the length are multiples of the
//!    region's page [OFW-4.93 lv1 a:0x226540-0x2265BC]. A failure gets
//!    [`DMA_UNMAP_ADDRESS_R3`] or [`DMA_UNMAP_LENGTH_R3`].
//! 6. It clears the mark and calls the device's entry at +184
//!    [OFW-4.93 lv1 a:0x2265C0-0x2265E8]. It returns that entry's r3
//!    sign-extended [OFW-4.93 lv1 a:0x226600].
//!
//! The entry at +208 [OFW-4.93 lv1 a:0x22709C] walks the device's region
//! list at +96 [OFW-4.93 lv1 a:0x2270AC-0x2270CC]. It gives the first
//! mapping of the first region that holds a mapping of the bus address
//! [OFW-4.93 lv1 a:0x2270D0-0x227104]. A mapping holds the bus address
//! when the address is in the mapping's range
//! [OFW-4.93 lv1 a:0x225BF8-0x225C1C] [OFW-4.93 lv1 a:0x225C3C-0x225C60]:
//! - The range starts at the region's bus address at +48 plus the
//!   mapping's offset at +40.
//! - The length of the range is at +56 of the mapping.
//!
//! The walk checks no partition. So a bus address in a region of lv1's
//! own partition finds lv1's mapping.
//!
//! The mapping function of `map_device_dma_region` stores 0 at +88 of
//! each mapping, as the capture [`DMA_MAP_CAPTURE`] reads. It also
//! checks the bus address and the length against the region's page. So
//! for a mapping of the partition, step 2 and step 5 do not fail.
//!
//! Device 0's entry at +184 [OFW-4.93 lv1 a:0x2279D4]
//! [OFW-4.93 lv1 a:0x35C930-0x35C934] [OFW-4.93 lv1 a:0x32D468-0x32D46C]
//! does these steps:
//! 1. It gets the I/O address of the bus address from the entry at +120,
//!    and returns that entry's failure at once
//!    [OFW-4.93 lv1 a:0x2279F8-0x227A28].
//! 2. It clears the IOPTEs with the entry at +136
//!    [OFW-4.93 lv1 a:0x227A2C-0x227AA8]. The call gives these inputs:
//!    - the I/O address;
//!    - the logical partition address at +80 of the mapping;
//!    - the device's I/O id at +120;
//!    - the flags 0x2000000000000000;
//!    - the length at +56 of the mapping and the region's page.
//! 3. When that entry gives 0, it unlinks the mapping from the region,
//!    destroys it and returns 0 [OFW-4.93 lv1 a:0x227AAC-0x227AC4]
//!    [OFW-4.93 lv1 a:0x22584C-0x2258B4].
//!
//! A failed clear also unlinks the mapping
//! [OFW-4.93 lv1 a:0x227AEC-0x227B30]. The entry returns 0 in place of
//! the failure only when all of these are true:
//! - the doubleword at r13 - 26600 is not 0;
//! - the partition id at +80 of the object that it points to is 1, and
//!   is not the caller's;
//! - the partition at +8 of the mapping is the caller's.
//!
//! The handler gives the id at +80 of that same object as the caller's
//! partition [OFW-4.93 lv1 a:0x3066AC-0x3066C8]. So for the hypercall, a
//! failed clear returns the failure.
//!
//! The map gave the entry at +120 the same bus address and the same
//! device word at +160. It gave the entry at +136 the same I/O address,
//! I/O id, logical partition address, length and page. The facts of
//! `map_device_dma_region` list the IOMMU state that each IOPTE write
//! reads; see [`is_game_os_dma_mapping`]. The flags of the clear set
//! neither bit 0 nor bit 1, so the clear skips the lookup that those bits
//! select.
//!
//! The function frees no logical partition address. So a page table
//! entry that maps the partition memory of the mapping does not change
//! the path.
//!
//! # The 4.93 kernel's calls of `unmap_device_dma_region`
//!
//! The kernel's unmap wrapper [OFW-4.93 lv2_kernel a:0x800000000007d430]
//! sends one call for each page of a range
//! [OFW-4.93 lv2_kernel a:0x800000000007d470-0x800000000007d4c4]. The
//! undo paths of the map wrapper send one call for each page that the
//! wrapper mapped [OFW-4.93 lv2_kernel a:0x800000000007d6dc-0x800000000007d73c]
//! [OFW-4.93 lv2_kernel a:0x800000000007d7b8-0x800000000007d818]. Each
//! call gives these inputs:
//! - r3 and r4: the bus id and the device id;
//! - r5: the bus address of the page;
//! - r6: the page size.
//!
//! The map wrapper maps each page with its own call, so each of the
//! kernel's unmap calls names the first byte and the length of one
//! mapping.
//!
//! [`DMA_MAP_CAPTURE`]: super::DMA_MAP_CAPTURE
//! [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX
//! [`is_game_os_dma_mapping`]: super::is_game_os_dma_mapping

use crate::lv1::status::{zero_extended, LV1_WRONG_STATE};

/// The handler of hypercall 177 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaUnmapHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `unmap_device_dma_region` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 177 in the live 4.93 hypercall
/// table ([`crate::lv1::table::LIVE_TABLE_HANDLERS`]).
pub const DMA_UNMAP_HANDLERS: &[DmaUnmapHandler] = &[DmaUnmapHandler {
    firmware: "4.93",
    handler: 0x30660C,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn dma_unmap_handler(firmware: &str) -> Option<&'static DmaUnmapHandler> {
    DMA_UNMAP_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `unmap_device_dma_region` for a bus id above
/// [`OPEN_DEVICE_BUS_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x30675C-0x30676C].
///
/// The high-word bits mark the inputs:
/// - bit 32 marks r3;
/// - bit 33 marks r4;
/// - bit 34 marks r5;
/// - bit 35 marks r6.
///
/// [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX
pub const DMA_UNMAP_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 for bus id 0: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x236420-0x236428] [OFW-4.93 lv1 a:0x236CFC-0x236D18].
pub const DMA_UNMAP_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 when the calling partition did not open the device:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x236C78].
pub const DMA_UNMAP_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 when no mapping in a region of the device holds the bus
/// address: `LV1_NO_ENTRY` (-6), with r5 marked
/// [OFW-4.93 lv1 a:0x226680-0x22668C] [OFW-4.93 lv1 a:0x236CB0-0x236CCC].
pub const DMA_UNMAP_NO_MAPPING_R3: u64 = 0x0000_0004_FFFF_FFFA;

/// The r3 for a bus address other than the first byte of the mapping
/// that holds it: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x226644-0x226650] [OFW-4.93 lv1 a:0x236CB0-0x236CCC].
pub const DMA_UNMAP_ADDRESS_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 for a length other than the length of the mapping:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r6 marked
/// [OFW-4.93 lv1 a:0x226634-0x226640] [OFW-4.93 lv1 a:0x236CB0-0x236CCC].
pub const DMA_UNMAP_LENGTH_R3: u64 = 0x0000_0008_FFFF_FFEF;
