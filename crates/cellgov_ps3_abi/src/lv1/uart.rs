//! The virtual UART of a partition:
//! - the handler of `configure_virtual_uart_irq` (hypercall 166);
//! - its argument checks and the r3 of each;
//! - the irq outlet that lv1 constructs, and the bitmap that it fills;
//! - the outlet id of the GameOS kernel's call;
//! - the handler of `get_virtual_uart_param` (hypercall 165), its
//!   checks, and the buffer sizes of the GameOS kernel's ports;
//! - the handler of `set_virtual_uart_param` (hypercall 164), its checks,
//!   and the interrupt status that lv1 computes after each call.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! The handler [OFW-4.93 lv1 a:0x2F2810] reads r3 and no other register
//! [OFW-4.93 lv1 a:0x2F2824]. r3 is the logical partition address of a
//! 32-byte bitmap. The handler returns the r3 that it builds in r31
//! [OFW-4.93 lv1 a:0x2F2A48]. Only a call that succeeds writes the return
//! slot of r4; see [`super::event_port`] for that slot. The vector's entry
//! zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r5 to r10 return 0, and r4
//! returns 0 for a call that fails.
//!
//! # The checks
//!
//! The handler checks r3 in this order, and the first failure is the
//! answer:
//! 1. r3 is not above [`VIRTUAL_UART_IRQ_ADDRESS_MAX`].
//! 2. A memory region holds r3. The handler looks up the region with the
//!    function [OFW-4.93 lv1 a:0x2CE74C] [OFW-4.93 lv1 a:0x2F28BC-0x2F28F8],
//!    as `configure_irq_state_bitmap` does; see [`super::interrupt`].
//! 3. The region's class at +72 is the class in the TOC slot at r2 - 5872
//!    [OFW-4.93 lv1 a:0x2F28FC-0x2F2910]. That slot and the slot that
//!    `configure_irq_state_bitmap` checks, r2 - 7048, hold the same class
//!    [OFW-4.93 lv1 a:0x358948] [OFW-4.93 lv1 a:0x3584B0].
//! 4. The region holds [`VIRTUAL_UART_IRQ_BITMAP_BYTES`] bytes from r3.
//! 5. The bitmap and its next byte are in one page of the region.
//! 6. The region has a page array at +176 [OFW-4.93 lv1 a:0x2F2948-0x2F2950].
//!
//! See [`CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3`] for the r3 of each
//! check.
//!
//! # The configure step
//!
//! The handler translates r3 to a real address through the region's page
//! array [OFW-4.93 lv1 a:0x2F2954-0x2F2984]. It then gives the function
//! [OFW-4.93 lv1 a:0x2F13FC] the partition's virtual UART interrupt
//! object, at +0x120 of the partition, and the real address
//! [OFW-4.93 lv1 a:0x2F2958-0x2F298C]. That function does these steps in
//! this order:
//! 1. It returns 5 when the object holds an outlet at +16
//!    [OFW-4.93 lv1 a:0x2F1474-0x2F1480]. The handler maps 5 to
//!    [`CONFIGURE_VIRTUAL_UART_IRQ_CONFIGURED_R3`].
//! 2. It allocates a 152-byte object and constructs it with the function
//!    [OFW-4.93 lv1 a:0x2DF6B8] [OFW-4.93 lv1 a:0x2F1484-0x2F14B0]. That
//!    function has the instructions and the TOC slots of the event receive
//!    port constructor [OFW-4.93 lv1 a:0x2DF60C-0x2DF6B4]
//!    [OFW-4.93 lv1 a:0x2DF6B8-0x2DF760]. So the outlet takes the first
//!    free id of the partition's object table, and has the port class and
//!    no connection; see [`super::event_port`].
//! 3. It checks that the region is live and is the caller's
//!    [OFW-4.93 lv1 a:0x2F1528-0x2F1568] [OFW-4.93 lv1 a:0x2F1790-0x2F17DC].
//! 4. It keeps the real address at +48, the region at +24 and the outlet
//!    at +16 of the object
//!    [OFW-4.93 lv1 a:0x2F15C4-0x2F15CC] [OFW-4.93 lv1 a:0x2F15E8-0x2F15EC].
//! 5. It copies the object's 32-byte shadow at +56 to the bitmap
//!    [OFW-4.93 lv1 a:0x2F15FC-0x2F1620].
//! 6. It puts the outlet in the slot of the object table that the
//!    outlet's id at +72 names [OFW-4.93 lv1 a:0x2F1624-0x2F16F0].
//! 7. It gives the outlet to the event function [OFW-4.93 lv1 a:0x2DF808]
//!    when the count of set shadow bits at +2200 is above 0
//!    [OFW-4.93 lv1 a:0x2F16F4-0x2F1708].
//!
//! The handler then stores the outlet's id in the return slot of r4, and
//! returns 0 [OFW-4.93 lv1 a:0x2F2A6C-0x2F2A8C].
//!
//! The outlet passes each check of `connect_irq_plug_ext`, as an irq
//! outlet of a logical SPE does; see [`super::spe`]. The constructor
//! stores the word in the TOC slot at r2 - 6032 at +136 of the outlet
//! [OFW-4.93 lv1 a:0x2F14AC] [OFW-4.93 lv1 a:0x2DF710]. That word
//! [OFW-4.93 lv1 a:0x3588A8] is not the word of a port
//! [OFW-4.93 lv1 a:0x358160]. So `set_spe_transition_notifier` refuses the
//! outlet, as it refuses an outlet of a logical SPE.
//!
//! # The shadow
//!
//! The object's init function [OFW-4.93 lv1 a:0x2F1258] sets these
//! fields:
//! - +16, the outlet, is 0 [OFW-4.93 lv1 a:0x2F12AC].
//! - +48 holds the address of the object's own doublewords at +88
//!   [OFW-4.93 lv1 a:0x2F1270] [OFW-4.93 lv1 a:0x2F12BC].
//! - +56 to +87, the shadow, are 0 [OFW-4.93 lv1 a:0x2F130C-0x2F1318].
//! - +2200, the count of set shadow bits, is 0 [OFW-4.93 lv1 a:0x2F1308].
//!
//! The raise step sets a shadow bit, adds 1 to the count, and stores the
//! shadow doubleword at the address that +48 holds
//! [OFW-4.93 lv1 a:0x2F225C-0x2F2278]. It does this only when the
//! endpoint's interrupt mask at +152 holds a bit of the event
//! [OFW-4.93 lv1 a:0x2F221C-0x2F2238]. The endpoint constructor sets the
//! mask to 0 [OFW-4.93 lv1 a:0x2F18DC]. The mask setter
//! [OFW-4.93 lv1 a:0x2F3F40-0x2F3F8C] is a step of the worker
//! [OFW-4.93 lv1 a:0x2F3C0C] of `set_virtual_uart_param` (hypercall 164),
//! which its handler calls [OFW-4.93 lv1 a:0x2F419C]. So the shadow and
//! the count stay 0 until the partition sets an endpoint's mask to a
//! value other than 0; see the section on the interrupt status.
//!
//! # The parameters of a port
//!
//! The handler of hypercall 165 [OFW-4.93 lv1 a:0x2F3A68] keeps r3 and r4
//! [OFW-4.93 lv1 a:0x2F3A78] [OFW-4.93 lv1 a:0x2F3A90]. It zeroes a stack
//! slot, and gives r3, r4 and the slot to the worker
//! [OFW-4.93 lv1 a:0x2F3AF0-0x2F3B04]. It then stores the slot in the
//! return slot of r4 [OFW-4.93 lv1 a:0x2F3B08-0x2F3B18], and returns the
//! worker's r3 [OFW-4.93 lv1 a:0x2F3B7C]. The worker writes the slot
//! only when it returns 0, and the vector's entry zeroes the return slots
//! [OFW-4.93 lv1 a:0x0C08]. So r4 returns 0 for a call that fails, and r5
//! to r10 return 0.
//!
//! The worker [OFW-4.93 lv1 a:0x2F34E0] does these steps in this order:
//! 1. It refuses r3 above [`VIRTUAL_UART_PORT_MAX`]
//!    [OFW-4.93 lv1 a:0x2F34E0] [OFW-4.93 lv1 a:0x2F34F8].
//! 2. It reads the port's endpoint from the slot of r3 in the table at
//!    +152 of the virtual UART object, at +0x120 of the partition
//!    [OFW-4.93 lv1 a:0x2F34FC-0x2F3580]. It refuses a slot with no
//!    endpoint [OFW-4.93 lv1 a:0x2F3584-0x2F35C0].
//! 3. It refuses r4 above [`VIRTUAL_UART_PARAM_MAX`]
//!    [OFW-4.93 lv1 a:0x2F35C4-0x2F35D8].
//! 4. It reads the parameter through a table of eight entries
//!    [OFW-4.93 lv1 a:0x2F35F0-0x2F3628], stores it in the slot, and
//!    returns 0.
//!
//! The endpoint's init function stores its buffer size at +112
//! [OFW-4.93 lv1 a:0x2EF650] [OFW-4.93 lv1 a:0x2EF68C].
//! `set_virtual_uart_param` sets only the parameters 0 to 2 and refuses
//! the others [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08]
//! [OFW-4.93 lv1 a:0x2F3DF0-0x2F3E04]. So no hypercall changes a buffer
//! size. lv1 makes the endpoints when it makes the partition, so no image
//! read gives their sizes; see [`GAME_OS_VIRTUAL_UART_PORTS`].
//!
//! The two sizes can change while the partition runs, through lv1's own
//! code and not through a hypercall of this group:
//! - The endpoint holds its peer in a weak reference at +56, whose target
//!   is at +72 [OFW-4.93 lv1 a:0x2EF734-0x2EF744]. A function links two
//!   endpoints as peers [OFW-4.93 lv1 a:0x2EFE6C-0x2EFEC0], and another
//!   unlinks them [OFW-4.93 lv1 a:0x2EFB50-0x2EFB60]. lv1's service code
//!   calls the link function [OFW-4.93 lv1 a:0x29A218]
//!   [OFW-4.93 lv1 a:0x2AA320] [OFW-4.93 lv1 a:0x2AA420]
//!   [OFW-4.93 lv1 a:0x2F5904].
//! - The word at +8 is the reference count, in steps of 4
//!   [OFW-4.93 lv1 a:0x2BED58-0x2BED6C] [OFW-4.93 lv1 a:0x2BEE78-0x2BEE8C].
//!   So parameter 5 gives 0 once the peer has no reference.
//! - The port function puts a new endpoint in the port's slot and
//!   releases the old one [OFW-4.93 lv1 a:0x2F1A0C-0x2F1A1C]. lv1's service
//!   code calls it [OFW-4.93 lv1 a:0x29A390] [OFW-4.93 lv1 a:0x2A99FC].
//!
//! No image read shows when that code runs. So the capture gives the sizes
//! only at the kernel's own calls; see [`GAME_OS_VIRTUAL_UART_PARAM_CALLS`].
//!
//! # Setting a parameter of a port
//!
//! The handler of hypercall 164 [OFW-4.93 lv1 a:0x2F4100] gives r3, r4
//! and r5 to the worker [OFW-4.93 lv1 a:0x2F4190-0x2F419C]. It returns
//! the worker's r3 [OFW-4.93 lv1 a:0x2F41A4] [OFW-4.93 lv1 a:0x2F4208],
//! and writes no return slot. The vector's entry zeroes the return slots
//! [OFW-4.93 lv1 a:0x0C08], so r4 to r10 return 0.
//!
//! The worker [OFW-4.93 lv1 a:0x2F3C0C] does these steps in this order:
//! 1. It refuses r3 above [`VIRTUAL_UART_PORT_MAX`]
//!    [OFW-4.93 lv1 a:0x2F3C0C] [OFW-4.93 lv1 a:0x2F3C30].
//! 2. It reads the port's endpoint from the slot of r3 in the table at
//!    +152 of the virtual UART object [OFW-4.93 lv1 a:0x2F3C34-0x2F3CB8].
//!    It refuses a slot with no endpoint [OFW-4.93 lv1 a:0x2F3CF4-0x2F3CF8].
//! 3. It refuses r4 above [`VIRTUAL_UART_SET_PARAM_MAX`]
//!    [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08] [OFW-4.93 lv1 a:0x2F3DF0-0x2F3E04].
//! 4. For the mask, it refuses r5 above [`VIRTUAL_UART_INTERRUPT_MASK_MAX`].
//! 5. It stores r5 in the endpoint's field of r4; see
//!    [`VIRTUAL_UART_PARAM_TX_TRIGGER`] for the fields.
//! 6. It calls each listener in the list at +48 of the endpoint
//!    [OFW-4.93 lv1 a:0x2F3D70-0x2F3DC4], and returns 0
//!    [OFW-4.93 lv1 a:0x2F3DC8-0x2F3DCC].
//!
//! The endpoint constructor sets the three fields to 0
//! [OFW-4.93 lv1 a:0x2F1864] [OFW-4.93 lv1 a:0x2F18D4-0x2F18DC]. The
//! image read has no other store to the three fields of an endpoint.
//! `get_virtual_uart_param` reads them for the parameters 0 to 2
//! [OFW-4.93 lv1 a:0x2F3670] [OFW-4.93 lv1 a:0x2F36F4]
//! [OFW-4.93 lv1 a:0x2F3748]. The port function can put a new endpoint in
//! the port's slot, as the section on the parameters of a port gives. So
//! the three values of a port can return to 0 at a time that no image
//! read shows.
//!
//! # The interrupt status
//!
//! The port function adds a listener to the endpoint that it puts in a
//! port's slot [OFW-4.93 lv1 a:0x2F1A50-0x2F1A70]. The listener holds the
//! virtual UART object and the status function
//! [OFW-4.93 lv1 a:0x2F0F18-0x2F0F94] [OFW-4.93 lv1 a:0x3588C0]
//! [OFW-4.93 lv1 a:0x349908]. The method that the worker calls
//! [OFW-4.93 lv1 a:0x3625F0-0x362604] [OFW-4.93 lv1 a:0x3495A8] calls the
//! status function with the object and the endpoint
//! [OFW-4.93 lv1 a:0x2F0AF4-0x2F0B38].
//!
//! The list at +48 of an endpoint holds only this listener:
//! - The port function's call is the only call of the listener's builder
//!   in the image read, and no word of the image holds the builder's
//!   descriptor [OFW-4.93 lv1 a:0x349728].
//! - The builder claims the slot at +160 of the endpoint. When the slot
//!   holds a listener, it removes the new listener from the list again
//!   [OFW-4.93 lv1 a:0x2F0FBC-0x2F1058].
//! - The other builders of the same form add their listener to an object
//!   that they make, and not to an endpoint
//!   [OFW-4.93 lv1 a:0x209E5C-0x209ED4] [OFW-4.93 lv1 a:0x22E0B4-0x22E12C]
//!   [OFW-4.93 lv1 a:0x2F55D8-0x2F5668].
//!
//! The status function [OFW-4.93 lv1 a:0x2F20E4] acts only when the
//! port's slot holds the endpoint [OFW-4.93 lv1 a:0x2F2128-0x2F2140]. It
//! sets these bits of the status:
//! - 1 when the endpoint has a peer with a reference, and the doubleword
//!   at +120 of the peer is not above the transmit trigger
//!   [OFW-4.93 lv1 a:0x2F21E0-0x2F2204] [OFW-4.93 lv1 a:0x2F2380-0x2F2384];
//! - 2 when the buffer size less the doubleword at +120 of the endpoint
//!   is not above the receive trigger
//!   [OFW-4.93 lv1 a:0x2F2208-0x2F2210] [OFW-4.93 lv1 a:0x2F233C-0x2F2348];
//! - 4 when the endpoint has no peer with a reference
//!   [OFW-4.93 lv1 a:0x2F21B0-0x2F21C8] [OFW-4.93 lv1 a:0x2F21E8]
//!   [OFW-4.93 lv1 a:0x2F23A4];
//! - 8 when the endpoint has a peer with a reference
//!   [OFW-4.93 lv1 a:0x2F21F4-0x2F21FC]. No mask that lv1 accepts holds
//!   this bit.
//!
//! The function then does one of these steps:
//! - When the byte at +128 of the endpoint is 0, and the status and the
//!   mask share a bit, it sets the port's shadow bit, as the section on
//!   the shadow gives [OFW-4.93 lv1 a:0x2F217C-0x2F2184]
//!   [OFW-4.93 lv1 a:0x2F221C-0x2F2278]. It gives the outlet to the event
//!   function when the count becomes 1
//!   [OFW-4.93 lv1 a:0x2F227C-0x2F2280] [OFW-4.93 lv1 a:0x2F2394-0x2F239C].
//! - Otherwise it clears the port's shadow bit, only when the bit is set
//!   [OFW-4.93 lv1 a:0x2F22DC-0x2F2334].
//!
//! So a call that leaves the mask at 0 sets no shadow bit, and changes no
//! byte while the shadow is 0. A call that leaves a mask other than 0 can
//! raise the port's interrupt. Whether it does depends on the peer and on
//! the doublewords at +120, which change with the port's traffic. The
//! kernel sets such a mask on each of its ports; see
//! [`VIRTUAL_UART_PORT_MASK_CAPTURE`].

use crate::hw::ppu_mmu::SMALL_PAGE_LOG2;

/// The handler of hypercall 166 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartIrqHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `configure_virtual_uart_irq` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 166 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_IRQ_HANDLERS: &[VirtualUartIrqHandler] = &[VirtualUartIrqHandler {
    firmware: "4.93",
    handler: 0x2F2810,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_irq_handler(firmware: &str) -> Option<&'static VirtualUartIrqHandler> {
    VIRTUAL_UART_IRQ_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest r3 that the handler looks up. It refuses an r3 above
/// 2^62 - 1, unsigned [OFW-4.93 lv1 a:0x2F289C-0x2F28A8].
pub const VIRTUAL_UART_IRQ_ADDRESS_MAX: u64 = 0x3FFF_FFFF_FFFF_FFFF;

/// The bytes of the bitmap.
///
/// The handler refuses a region with fewer than 32 bytes from r3 to its
/// end [OFW-4.93 lv1 a:0x2F2914-0x2F2924]. lv1 copies 32 bytes of its
/// shadow to the bitmap [OFW-4.93 lv1 a:0x2F1604-0x2F1620].
pub const VIRTUAL_UART_IRQ_BITMAP_BYTES: u64 = 32;

/// Whether the bitmap at `address` and its next byte are in one 4 KB page.
///
/// The handler takes the region's page size from the exponent at +160
/// [OFW-4.93 lv1 a:0x2F290C]. It refuses a bitmap whose first byte and
/// the byte after its end are in two pages
/// [OFW-4.93 lv1 a:0x2F2928-0x2F2944]. It tests the offset of r3 from the
/// region's base [OFW-4.93 lv1 a:0x2F2914]. Each PPE page size is
/// a multiple of 4 KB [CBE-Handbook p:88 s:4.2.6.6]. So for a region whose
/// base is 4 KB-aligned, such as the real mode region at 0, a bitmap that
/// passes this test passes lv1's test for each page size.
pub const fn virtual_uart_irq_bitmap_in_small_page(address: u64) -> bool {
    let page = !((1u64 << SMALL_PAGE_LOG2) - 1);
    address & page == address.wrapping_add(VIRTUAL_UART_IRQ_BITMAP_BYTES) & page
}

/// The r3 of `configure_virtual_uart_irq` for an r3 that no memory region
/// of the partition holds: `LV1_NO_ENTRY` (-6), with r3 marked in the high
/// word [OFW-4.93 lv1 a:0x2F2AB8-0x2F2AC4].
///
/// As with `allocate_memory`, bit 32 marks r3. The handler returns this
/// r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_IRQ_ADDRESS_MAX`];
/// - no region holds r3 [OFW-4.93 lv1 a:0x2F28F8];
/// - the region has no page array.
///
/// Past the checks, these answers depend on lv1's own state:
/// - The live check of step 3 returns this r3
///   [OFW-4.93 lv1 a:0x2F17C8-0x2F17D8] [OFW-4.93 lv1 a:0x2F29BC-0x2F29D4].
/// - A failed allocation returns `LV1_RESOURCE_SHORTAGE` (-2),
///   zero-extended [OFW-4.93 lv1 a:0x2F1494-0x2F14A0]
///   [OFW-4.93 lv1 a:0x2F2AA0-0x2F2AB4].
pub const CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `configure_virtual_uart_irq` for a region that fails one of
/// these checks: `LV1_TYPE_MISMATCH` (-8), with r3 marked
/// [OFW-4.93 lv1 a:0x2F2ACC-0x2F2AD8]:
/// - the region's class;
/// - the [`VIRTUAL_UART_IRQ_BITMAP_BYTES`] bytes from r3 to the region's
///   end.
pub const CONFIGURE_VIRTUAL_UART_IRQ_REGION_R3: u64 = 0x0000_0001_FFFF_FFF8;

/// The r3 of `configure_virtual_uart_irq` for a bitmap whose first byte
/// and the byte after its end are in two pages:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked
/// [OFW-4.93 lv1 a:0x2F2AE0-0x2F2AEC].
pub const CONFIGURE_VIRTUAL_UART_IRQ_PAGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 of `configure_virtual_uart_irq` for a partition whose virtual
/// UART already has an irq outlet: `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x2F2990-0x2F2994] [OFW-4.93 lv1 a:0x2F2A90-0x2F2A98].
pub const CONFIGURE_VIRTUAL_UART_IRQ_CONFIGURED_R3: u64 = 0x0000_0000_FFFF_FFF5;

/// The console capture of the outlet id of the 4.93 kernel's call.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const VIRTUAL_UART_IRQ_OUTLET_CAPTURE: &str = "tests/micro/lv2_vuart_irq_outlet";

/// The `configure_virtual_uart_irq` call of the GameOS kernel's boot at
/// one firmware, and the outlet id that lv1 gave it on the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsVirtualUartIrqCall {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the call's `sc`.
    pub sc: u64,
    /// The outlet id in r4 of lv1's answer.
    pub outlet: u64,
}

/// The kernel's call for each firmware that a source gives the outlet id
/// for.
///
/// The partition's other objects decide the id, so no image read gives
/// it. The 4.93 kernel makes the call in the function
/// [OFW-4.93 lv2_kernel a:0x8000000000017ad0], and calls that function
/// again until it returns 0
/// [OFW-4.93 lv2_kernel a:0x8000000000017cac-0x8000000000017cb8]. The
/// capture [`VIRTUAL_UART_IRQ_OUTLET_CAPTURE`] reads the id where the
/// kernel stores it after r3 = 0
/// [OFW-4.93 lv2_kernel a:0x8000000000017b10-0x8000000000017b20].
pub const GAME_OS_VIRTUAL_UART_IRQ_CALLS: &[GameOsVirtualUartIrqCall] =
    &[GameOsVirtualUartIrqCall {
        firmware: "4.93",
        sc: 0x8000_0000_0001_7b0c,
        outlet: 18,
    }];

/// The outlet id that lv1 gave the GameOS kernel's call at the `sc`
/// address `sc`.
///
/// The value is `None` when no source gives that id at the firmware
/// version `firmware`.
pub fn game_os_virtual_uart_outlet(firmware: &str, sc: u64) -> Option<u64> {
    GAME_OS_VIRTUAL_UART_IRQ_CALLS
        .iter()
        .find(|call| call.firmware == firmware && call.sc == sc)
        .map(|call| call.outlet)
}

/// The handler of hypercall 165 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartParamHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `get_virtual_uart_param` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 165 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_PARAM_HANDLERS: &[VirtualUartParamHandler] = &[VirtualUartParamHandler {
    firmware: "4.93",
    handler: 0x2F3A68,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_param_handler(firmware: &str) -> Option<&'static VirtualUartParamHandler> {
    VIRTUAL_UART_PARAM_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest port number. The worker refuses an r3 above 255, unsigned
/// [OFW-4.93 lv1 a:0x2F34E0] [OFW-4.93 lv1 a:0x2F34F8].
pub const VIRTUAL_UART_PORT_MAX: u64 = 0xFF;

/// The largest parameter id. The worker refuses an r4 above 7, unsigned
/// [OFW-4.93 lv1 a:0x2F35C4-0x2F35C8].
pub const VIRTUAL_UART_PARAM_MAX: u64 = 7;

/// The parameter id of the port's receive buffer size,
/// `PARAM_RX_BUF_SIZE` in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker's entry for 3 reads +112 of the port's endpoint
/// [OFW-4.93 lv1 a:0x2F3758-0x2F3764].
pub const VIRTUAL_UART_PARAM_RX_BUFFER_SIZE: u64 = 3;

/// The parameter id of the port's transmit buffer size,
/// `PARAM_TX_BUF_SIZE` in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker's entry for 5 reads +112 of the endpoint at +72 of the
/// port's endpoint: the receive buffer size of the peer that the port
/// sends to. It gives the value 0, with r3 = 0, when the port has no peer,
/// or when bits 2 to 31 of the word at +8 of the peer are 0
/// [OFW-4.93 lv1 a:0x2F37BC-0x2F3828] [OFW-4.93 lv1 a:0x2F399C-0x2F39A0].
/// So the value changes when lv1 links the port to another peer.
pub const VIRTUAL_UART_PARAM_TX_BUFFER_SIZE: u64 = 5;

/// The r3 of `get_virtual_uart_param` for a port that lv1 does not hold:
/// `LV1_NO_ENTRY` (-6), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F3958-0x2F3968].
///
/// The worker returns this r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`];
/// - the table holds no endpoint for r3.
pub const GET_VIRTUAL_UART_PARAM_NO_PORT_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `get_virtual_uart_param` for a parameter id above
/// [`VIRTUAL_UART_PARAM_MAX`]: `LV1_NO_ENTRY` (-6), with r4 marked in the
/// high word [OFW-4.93 lv1 a:0x2F35CC-0x2F35D8].
pub const GET_VIRTUAL_UART_PARAM_NO_PARAM_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The console capture of the buffer sizes of the 4.93 kernel's ports.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const VIRTUAL_UART_PORT_CAPTURE: &str = "tests/micro/lv2_vuart_port_params";

/// A port of the GameOS partition's virtual UART at one firmware, and
/// the buffer sizes that lv1 gave the kernel's calls on the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsVirtualUartPort {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The port number in r3 of the call.
    pub port: u64,
    /// The value of [`VIRTUAL_UART_PARAM_RX_BUFFER_SIZE`].
    pub rx_buffer_size: u64,
    /// The value of [`VIRTUAL_UART_PARAM_TX_BUFFER_SIZE`].
    pub tx_buffer_size: u64,
}

/// The ports that the GameOS kernel opens, for each firmware that a source
/// gives them for, by ascending port number.
///
/// The 4.93 kernel's open function reads both sizes with hypercall 165
/// [OFW-4.93 lv2_kernel a:0x8000000000017708-0x8000000000017750]. It stores
/// them in its port record only after r3 = 0
/// [OFW-4.93 lv2_kernel a:0x8000000000017740]
/// [OFW-4.93 lv2_kernel a:0x800000000001775c]. The capture
/// [`VIRTUAL_UART_PORT_CAPTURE`] reads them from the records of the
/// three ports that the kernel opened.
pub const GAME_OS_VIRTUAL_UART_PORTS: &[GameOsVirtualUartPort] = &[
    GameOsVirtualUartPort {
        firmware: "4.93",
        port: 0,
        rx_buffer_size: 0x800,
        tx_buffer_size: 0x800,
    },
    GameOsVirtualUartPort {
        firmware: "4.93",
        port: 2,
        rx_buffer_size: 0x800,
        tx_buffer_size: 0x800,
    },
    GameOsVirtualUartPort {
        firmware: "4.93",
        port: 10,
        rx_buffer_size: 0x1000,
        tx_buffer_size: 0x1000,
    },
];

/// The GameOS partition's port `port` at the firmware version `firmware`.
///
/// The value is `None` when no source gives that port at `firmware`.
pub fn game_os_virtual_uart_port(
    firmware: &str,
    port: u64,
) -> Option<&'static GameOsVirtualUartPort> {
    GAME_OS_VIRTUAL_UART_PORTS
        .iter()
        .find(|row| row.firmware == firmware && row.port == port)
}

/// A `get_virtual_uart_param` call of the GameOS kernel's open function at
/// one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsVirtualUartParamCall {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the call's `sc`.
    pub sc: u64,
    /// The parameter id in r4 of the call.
    pub param: u64,
}

/// The kernel's calls for the buffer sizes, for each firmware that a
/// source gives them for, by ascending `sc` address.
///
/// The capture [`VIRTUAL_UART_PORT_CAPTURE`] reads lv1's answers to these
/// calls, and to no other call. The 4.93 kernel's open function sends 165
/// with r4 = 3 [OFW-4.93 lv2_kernel a:0x8000000000017714-0x8000000000017730],
/// then with r4 = 5
/// [OFW-4.93 lv2_kernel a:0x8000000000017744-0x8000000000017750].
pub const GAME_OS_VIRTUAL_UART_PARAM_CALLS: &[GameOsVirtualUartParamCall] = &[
    GameOsVirtualUartParamCall {
        firmware: "4.93",
        sc: 0x8000_0000_0001_7730,
        param: VIRTUAL_UART_PARAM_RX_BUFFER_SIZE,
    },
    GameOsVirtualUartParamCall {
        firmware: "4.93",
        sc: 0x8000_0000_0001_7750,
        param: VIRTUAL_UART_PARAM_TX_BUFFER_SIZE,
    },
];

/// Whether the GameOS kernel's open function asks for parameter `param` at
/// the `sc` address `sc` at the firmware version `firmware`.
pub fn is_game_os_virtual_uart_param_call(firmware: &str, sc: u64, param: u64) -> bool {
    GAME_OS_VIRTUAL_UART_PARAM_CALLS
        .iter()
        .any(|call| call.firmware == firmware && call.sc == sc && call.param == param)
}

/// The handler of hypercall 164 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartSetParamHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `set_virtual_uart_param` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 164 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_SET_PARAM_HANDLERS: &[VirtualUartSetParamHandler] =
    &[VirtualUartSetParamHandler {
        firmware: "4.93",
        handler: 0x2F4100,
    }];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_set_param_handler(
    firmware: &str,
) -> Option<&'static VirtualUartSetParamHandler> {
    VIRTUAL_UART_SET_PARAM_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The parameter id of the port's transmit trigger, `PARAM_TX_TRIGGER` in
/// Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker stores r5 for this id at +136 of the endpoint
/// [OFW-4.93 lv1 a:0x2F3D0C-0x2F3D4C].
pub const VIRTUAL_UART_PARAM_TX_TRIGGER: u64 = 0;

/// The parameter id of the port's receive trigger, `PARAM_RX_TRIGGER` in
/// Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker stores r5 for this id at +144 of the endpoint
/// [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D00] [OFW-4.93 lv1 a:0x2F3E58-0x2F3E94].
pub const VIRTUAL_UART_PARAM_RX_TRIGGER: u64 = 1;

/// The parameter id of the port's interrupt mask, `PARAM_INTERRUPT_MASK`
/// in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker stores r5 for this id at +152 of the endpoint
/// [OFW-4.93 lv1 a:0x2F3DF0-0x2F3DF4] [OFW-4.93 lv1 a:0x2F3F4C-0x2F3F8C].
pub const VIRTUAL_UART_PARAM_INTERRUPT_MASK: u64 = 2;

/// The largest parameter id that `set_virtual_uart_param` sets. The
/// worker refuses an r4 above 2, unsigned
/// [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08] [OFW-4.93 lv1 a:0x2F3DF0-0x2F3DF8].
pub const VIRTUAL_UART_SET_PARAM_MAX: u64 = VIRTUAL_UART_PARAM_INTERRUPT_MASK;

/// The largest interrupt mask. The worker refuses an r5 with a bit above
/// bit 2 for the mask [OFW-4.93 lv1 a:0x2F3F40-0x2F3F48]. The status
/// bits that a mask can match are 1, 2 and 4; see [`crate::lv1::uart`].
pub const VIRTUAL_UART_INTERRUPT_MASK_MAX: u64 = 7;

/// The r3 of `set_virtual_uart_param` for a port that lv1 does not hold:
/// `LV1_NO_ENTRY` (-6), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F4038-0x2F4044].
///
/// The worker returns this r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`];
/// - the table holds no endpoint for r3.
pub const SET_VIRTUAL_UART_PARAM_NO_PORT_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `set_virtual_uart_param` for a parameter id above
/// [`VIRTUAL_UART_SET_PARAM_MAX`]: `LV1_NO_ENTRY` (-6), with r4 marked in
/// the high word [OFW-4.93 lv1 a:0x2F3DF8-0x2F3E04].
pub const SET_VIRTUAL_UART_PARAM_NO_PARAM_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `set_virtual_uart_param` for a mask above
/// [`VIRTUAL_UART_INTERRUPT_MASK_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE`
/// (-17), with r5 marked in the high word
/// [OFW-4.93 lv1 a:0x2F40EC-0x2F40F8].
pub const SET_VIRTUAL_UART_PARAM_MASK_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The console capture of the interrupt mask that the 4.93 kernel keeps
/// for each of its ports.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const VIRTUAL_UART_PORT_MASK_CAPTURE: &str = "tests/micro/lv2_vuart_port_mask";

#[cfg(test)]
#[path = "tests/uart_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/uart_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/uart_param_tests.rs"]
mod param_tests;

#[cfg(test)]
#[path = "tests/uart_param_console_tests.rs"]
mod param_console_tests;

#[cfg(test)]
#[path = "tests/uart_set_param_tests.rs"]
mod set_param_tests;

#[cfg(test)]
#[path = "tests/uart_set_param_console_tests.rs"]
mod set_param_console_tests;
