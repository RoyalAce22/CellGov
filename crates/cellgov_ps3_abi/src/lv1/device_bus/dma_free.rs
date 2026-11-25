//! The facts of `free_device_dma_region`: the handler, its checks and
//! the 4.93 kernel's calls.
//!
//! # `free_device_dma_region`
//!
//! The handler [OFW-4.93 lv1 a:0x306918] keeps r3 to r5
//! [OFW-4.93 lv1 a:0x30692C] [OFW-4.93 lv1 a:0x306944-0x306948] and does
//! these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`DMA_FREE_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x3069A8-0x3069AC]
//!    [OFW-4.93 lv1 a:0x306A58-0x306A68].
//! 2. It calls the free function [OFW-4.93 lv1 a:0x236E3C] with the
//!    caller's partition and r3 to r5 [OFW-4.93 lv1 a:0x3069B0-0x3069CC].
//! 3. It returns that function's r3 [OFW-4.93 lv1 a:0x3069D0]
//!    [OFW-4.93 lv1 a:0x306A38]. The handler writes no other slot, and
//!    the vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4
//!    to r10 return 0.
//!
//! The free function does these steps. The first failure is the answer:
//! 1. The access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x236E7C-0x236E84], which
//!    [`network`](crate::lv1::network) describes. Bus 0 gets
//!    [`DMA_FREE_NO_BUS_R3`]. A partition that did not open the device
//!    gets [`DMA_FREE_NOT_OPEN_R3`]. The function zero-extends a failed
//!    status and sets bit (mark + 31) for a mark other than 0
//!    [OFW-4.93 lv1 a:0x236EC8-0x236EE4]. For bus 1, the check first
//!    calls [OFW-4.93 lv1 a:0x235D40] with the device id, and a 0 return
//!    gets -6 with mark 2 [OFW-4.93 lv1 a:0x2364A0-0x2364B4]
//!    [OFW-4.93 lv1 a:0x236464-0x236474].
//! 2. The device lookup [OFW-4.93 lv1 a:0x236E88-0x236E90]. A miss gives
//!    -6 sign-extended, with no mark [OFW-4.93 lv1 a:0x236E94-0x236EA0].
//!    For devices 0, 1 and 2 of bus 1, the lookup finds the device that
//!    `map_device_dma_region` finds.
//! 3. For device 1 or 2 of bus 1, it calls the slot free function
//!    [OFW-4.93 lv1 a:0x23D644] with the device's slot object
//!    [OFW-4.93 lv1 a:0x236EA4-0x236F34]. For each other device, it calls
//!    the device's free function [OFW-4.93 lv1 a:0x22622C] with the
//!    device, the partition, r5 and a mark
//!    [OFW-4.93 lv1 a:0x236EAC-0x236EBC]. On 0 the function returns 0
//!    [OFW-4.93 lv1 a:0x236F60-0x236F84]. A nonzero r3 gets the treatment
//!    of step 1 [OFW-4.93 lv1 a:0x236EC0-0x236EE4]
//!    [OFW-4.93 lv1 a:0x236F38-0x236F5C].
//!
//! The slot object of device 1 is entry 0 of the table that
//! `allocate_device_dma_region` reads, and the slot object of device 2 is
//! entry 1 [OFW-4.93 lv1 a:0x236F0C-0x236F24] [OFW-4.93 lv1 a:0x23D0F8-0x23D104].
//!
//! The device's free function holds the device's lock at +88
//! [OFW-4.93 lv1 a:0x226258-0x22627C] and does these steps:
//! 1. It finds the region that holds the bus address with the device's
//!    entry at +200 [OFW-4.93 lv1 a:0x226280-0x2262A0]
//!    [OFW-4.93 lv1 a:0x35C940-0x35C944] [OFW-4.93 lv1 a:0x32D450-0x32D454].
//!    That entry walks the device's region list at +96 and checks no
//!    partition, as the facts of `map_device_dma_region` give. When no
//!    region holds the address, the answer is [`DMA_FREE_NO_REGION_R3`]
//!    [OFW-4.93 lv1 a:0x2263BC-0x2263C8].
//! 2. A bus address other than the region's address at +48 gets
//!    [`DMA_FREE_ADDRESS_R3`] [OFW-4.93 lv1 a:0x2262AC-0x2262B4]
//!    [OFW-4.93 lv1 a:0x22635C-0x226364].
//! 3. It clears the mark and calls the device's entry at +192, the free
//!    method [OFW-4.93 lv1 a:0x2262B8-0x2262D8]
//!    [OFW-4.93 lv1 a:0x35C938-0x35C93C] [OFW-4.93 lv1 a:0x32D408-0x32D40C].
//!    When the method returns a nonzero r3, the function stores mark 3
//!    [OFW-4.93 lv1 a:0x22631C-0x226320]. The function returns the
//!    method's r3 sign-extended [OFW-4.93 lv1 a:0x2262F8]
//!    [OFW-4.93 lv1 a:0x226338].
//!
//! The free method [OFW-4.93 lv1 a:0x227650] does these steps:
//! 1. A region that is not in the list at +96 gets -6
//!    [OFW-4.93 lv1 a:0x227660-0x227698]. The region of step 1 is in that
//!    list.
//! 2. A region that holds a mapping gets `LV1_BUSY` (-9)
//!    [OFW-4.93 lv1 a:0x2276B8-0x2276C4]. The doubleword at +72 of the
//!    region is the head of its mapping list
//!    [OFW-4.93 lv1 a:0x225860-0x225898]. So the free answers
//!    [`DMA_FREE_BUSY_R3`].
//! 3. Otherwise it releases the region's bus address through the device's
//!    method at +112, unlinks the region from the list at +96, destroys
//!    it and returns 0 [OFW-4.93 lv1 a:0x2276C8-0x22773C]. It does not
//!    read the r3 of the release.
//!
//! The find and the method check no partition. So the address of a
//! region of lv1's own partition frees lv1's region when that region
//! holds no mapping, and another address in that region gets
//! [`DMA_FREE_ADDRESS_R3`].
//!
//! The release is the release that `close_device` makes for each region
//! of the partition. The facts of `close_device` give what it does to the
//! device's pool. No read shows the address that the pool gives after the
//! release.
//!
//! The slot free function holds the lock at +0 of the slot object
//! [OFW-4.93 lv1 a:0x23D678-0x23D698] and does these steps:
//! 1. It clears the mark and calls the device's free function for the
//!    device at +24 of the slot object, with the partition and the bus
//!    address [OFW-4.93 lv1 a:0x23D69C-0x23D6B4]. A nonzero r3 is the
//!    answer, sign-extended, with the mark that the function stored
//!    [OFW-4.93 lv1 a:0x23D6B8-0x23D6C0] [OFW-4.93 lv1 a:0x23D798-0x23D7AC].
//! 2. It searches the six slots at +104, in order, for a marked slot that
//!    holds the partition and the low word of the bus address
//!    [OFW-4.93 lv1 a:0x23D6C4-0x23D70C]. It calls the function that
//!    writes the slot's registers [OFW-4.93 lv1 a:0x23D3A8] with 0 in r5
//!    to r7 [OFW-4.93 lv1 a:0x23D710-0x23D724].
//! 3. It searches again, clears the mark of the slot that it finds and
//!    calls that function again [OFW-4.93 lv1 a:0x23D728-0x23D794]. It
//!    then returns 0.
//! 4. When a search finds no slot, it returns -1
//!    [OFW-4.93 lv1 a:0x23D7D4-0x23D7EC]. The region is freed by then.
//!
//! `allocate_device_dma_region` marks a slot with the partition and the
//! low word of the address of each region that it gives device 1 or 2.
//! The facts of that call give the slot function. So a search for a
//! region of the partition finds the region's slot.
//!
//! # The 4.93 kernel's calls of `free_device_dma_region`
//!
//! The kernel's wrapper [OFW-4.93 lv2_kernel a:0x8000000000095758] sends
//! the call with r3 to r5 of its caller
//! [OFW-4.93 lv2_kernel a:0x8000000000095758-0x8000000000095764]:
//! - r3 and r4: the bus id and the device id;
//! - r5: the bus address of the region.
//!
//! No direct call path from the kernel's entries reaches the wrapper.
//!
//! [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX

use crate::lv1::status::{zero_extended, LV1_WRONG_STATE};

/// The handler of hypercall 175 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaFreeHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `free_device_dma_region` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 175 in the live 4.93 hypercall
/// table ([`crate::lv1::table::LIVE_TABLE_HANDLERS`]).
pub const DMA_FREE_HANDLERS: &[DmaFreeHandler] = &[DmaFreeHandler {
    firmware: "4.93",
    handler: 0x306918,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn dma_free_handler(firmware: &str) -> Option<&'static DmaFreeHandler> {
    DMA_FREE_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `free_device_dma_region` for a bus id above
/// [`OPEN_DEVICE_BUS_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x306A58-0x306A64].
///
/// The high-word bits mark the inputs:
/// - bit 32 marks r3;
/// - bit 33 marks r4;
/// - bit 34 marks r5.
///
/// [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX
pub const DMA_FREE_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 for bus id 0: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x236420-0x236428] [OFW-4.93 lv1 a:0x236EC8-0x236EE4].
pub const DMA_FREE_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 when the calling partition did not open the device:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x236EC8-0x236ED4].
pub const DMA_FREE_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 when no region of the device holds the bus address:
/// `LV1_NO_ENTRY` (-6), with r5 marked
/// [OFW-4.93 lv1 a:0x2263BC-0x2263C4] [OFW-4.93 lv1 a:0x236EC8-0x236EE4].
pub const DMA_FREE_NO_REGION_R3: u64 = 0x0000_0004_FFFF_FFFA;

/// The r3 for a bus address other than the address of the region that
/// holds it: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x22635C-0x226364] [OFW-4.93 lv1 a:0x236EC8-0x236EE4].
pub const DMA_FREE_ADDRESS_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 for a region that holds a mapping: `LV1_BUSY` (-9), with r5
/// marked [OFW-4.93 lv1 a:0x2276B8-0x2276C4]
/// [OFW-4.93 lv1 a:0x22631C-0x226320] [OFW-4.93 lv1 a:0x236EC8-0x236EE4].
pub const DMA_FREE_BUSY_R3: u64 = 0x0000_0004_FFFF_FFF7;
