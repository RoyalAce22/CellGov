//! The virtual UART interrupt of a partition:
//! - the handler of `configure_virtual_uart_irq` (hypercall 166);
//! - its argument checks and the r3 of each;
//! - the irq outlet that lv1 constructs, and the bitmap that it fills;
//! - the outlet id of the GameOS kernel's call.
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
//! the count stay 0 until the partition sets an endpoint's mask.

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

#[cfg(test)]
#[path = "tests/uart_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/uart_console_tests.rs"]
mod console_tests;
