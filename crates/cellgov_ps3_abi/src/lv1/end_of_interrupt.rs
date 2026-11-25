//! The end of an interrupt on a plug of a logical PPE thread:
//! - the handler of `end_of_interrupt_ext` (hypercall 30);
//! - its argument checks and the r3 of each;
//! - the port's end of interrupt, which clears the pending byte and the
//!   plug's status bit;
//! - the node list of the port, and the virtual UART's node.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! The handler [OFW-4.93 lv1 a:0x2E9710] reads three inputs and no other
//! register:
//! - r3 is a logical PPE id;
//! - r4 is a hardware thread of that PPE;
//! - r5 is a plug number of that thread.
//!
//! The handler returns the r3 that it builds in r31
//! [OFW-4.93 lv1 a:0x2E9940]. Neither the handler nor a function that it
//! calls writes a return slot. The vector's entry zeroes the slots
//! [OFW-4.93 lv1 a:0x0C08], so r4 to r10 return 0.
//!
//! # The checks
//!
//! The handler checks the arguments in this order, and the first failure
//! is the answer:
//! 1. r3 names a live logical PPE: the object table holds an object at
//!    r3 [OFW-4.93 lv1 a:0x2E97A4-0x2E98B8], its class word at +48 is the
//!    class in the TOC slot at r2 - 6984 [OFW-4.93 lv1 a:0x2E98BC-0x2E98C8],
//!    and its word at +188 is 1 [OFW-4.93 lv1 a:0x2E99A8-0x2E99B0]. See
//!    [`END_OF_INTERRUPT_PPE_ID_R3`].
//! 2. r4 is below the count that the function [OFW-4.93 lv1 a:0x2BF2FC]
//!    returns, 2. See [`END_OF_INTERRUPT_THREAD_R3`].
//! 3. r5 is not above 0xFF. See [`END_OF_INTERRUPT_PLUG_R3`].
//!
//! The handler then gives the PPE object, r4 and r5 to the function
//! [OFW-4.93 lv1 a:0x2E0B64] [OFW-4.93 lv1 a:0x2E99CC-0x2E99D8]. It maps
//! the status of that function to r3 [OFW-4.93 lv1 a:0x2E99DC-0x2E9A5C]:
//! - 0 gives r3 = 0.
//! - 0x96095A03 or 0x96095A04 gives [`END_OF_INTERRUPT_WRONG_STATE_R3`].
//! - 0x96095A05 gives [`END_OF_INTERRUPT_NOT_PENDING_R3`].
//! - Any other status gives `LV1_INTERNAL_ERROR`, zero-extended. No
//!   function of the chain below returns another status.
//!
//! # The end of interrupt
//!
//! The function [OFW-4.93 lv1 a:0x2E0B64] takes the thread's plug table
//! at +144 + 8 * r4 of the PPE object; see [`super::irq_plug`]. It gives
//! the table and the plug to the function [OFW-4.93 lv1 a:0x2DFEA8]. That
//! function returns 0x96095A04 for a plug whose slot at +16 + 8 * plug
//! holds no port [OFW-4.93 lv1 a:0x2DFEA8-0x2DFEC0]. Otherwise it gives
//! the port to the port's end of interrupt [OFW-4.93 lv1 a:0x2DFC5C]
//! [OFW-4.93 lv1 a:0x2DFEC4-0x2DFEC8].
//!
//! The port's end of interrupt takes the port's lock at +88 and does
//! these steps:
//! 1. It returns 0x96095A03 for a port whose byte at +148 is 0
//!    [OFW-4.93 lv1 a:0x2DFCB8-0x2DFCC8].
//! 2. It returns 0x96095A05 for a port whose pending byte at +149 is 0
//!    [OFW-4.93 lv1 a:0x2DFCCC-0x2DFCD4] [OFW-4.93 lv1 a:0x2DFE34-0x2DFE38].
//! 3. It returns 0x96095A04 for a port with no connection at +128
//!    [OFW-4.93 lv1 a:0x2DFCD8-0x2DFCE8].
//! 4. It clears the pending byte [OFW-4.93 lv1 a:0x2DFCF8].
//! 5. Under the lock that +0 of the plug table points to, it gives the
//!    thread's lv1 context at +8 of the table and the plug at +144 of the
//!    port to the function [OFW-4.93 lv1 a:0x2C3794]
//!    [OFW-4.93 lv1 a:0x2DFD00-0x2DFD3C]. That function clears the plug's
//!    bit in the status shadow, and stores the shadow doubleword through
//!    +1728 of the context; see [`super::irq_plug::plug_status_bit`]. The
//!    connect step of `connect_irq_plug_ext` uses the same function.
//! 6. It releases both locks [OFW-4.93 lv1 a:0x2DFD40-0x2DFD84].
//! 7. It calls a method of each node of the list at +104 of the port,
//!    with the node's object at +56 and the port
//!    [OFW-4.93 lv1 a:0x2DFD88-0x2DFDDC]. The method is the slot at +16 of
//!    the vtable that +56 of the node holds.
//! 8. It returns 0 [OFW-4.93 lv1 a:0x2DFDE0].
//!
//! The event function [OFW-4.93 lv1 a:0x2DF808] sets the pending byte,
//! and returns early while it is set; see [`super::send_event`]. So this
//! call lets the next event raise the plug again.
//!
//! # The node list
//!
//! Each port constructor empties the list at +104
//! [OFW-4.93 lv1 a:0x2DF65C] [OFW-4.93 lv1 a:0x2DF708]. The lists of the
//! objects that CellGov holds are these:
//! - A port of `construct_event_receive_port`: the function of the call
//!   links no node after the constructor
//!   [OFW-4.93 lv1 a:0x2E2544-0x2E2650]. So the list is empty.
//! - An irq outlet of a logical SPE: the outlet constructor gives +104 of
//!   the outlet to a list insert [OFW-4.93 lv1 a:0x2CB274-0x2CB2A4]; see
//!   [`super::spe`]. So the list holds a node.
//! - The irq outlet of the virtual UART: the configure step of
//!   `configure_virtual_uart_irq` links one 88-byte node to +104 of the
//!   outlet [OFW-4.93 lv1 a:0x2F14C0-0x2F151C]; see [`super::uart`].
//!
//! # The virtual UART's node
//!
//! The configure step stores these values in the node's object at +56
//! [OFW-4.93 lv1 a:0x2F14EC-0x2F1500]:
//! - +0 holds the vtable in the TOC slot at r2 - 6024
//!   [OFW-4.93 lv1 a:0x3588B0]. The vtable's slot at +16
//!   [OFW-4.93 lv1 a:0x362650] holds the function descriptor
//!   [OFW-4.93 lv1 a:0x3495C0] of the method [OFW-4.93 lv1 a:0x2F0B78].
//! - +8 holds the interrupt object of the partition's virtual UART.
//! - +16 holds the function descriptor in the TOC slot at r2 - 6016
//!   [OFW-4.93 lv1 a:0x3588B8]. That descriptor [OFW-4.93 lv1 a:0x349800]
//!   names the function [OFW-4.93 lv1 a:0x2F118C].
//! - +24 holds 0.
//!
//! The method calls the function at +16 with the object at +8 and the
//! port [OFW-4.93 lv1 a:0x2F0B78-0x2F0BF8]. That function takes the lock
//! at +8 of that interrupt object. It gives the port to the
//! event function when the count of set shadow bits at +2200 is above 0
//! [OFW-4.93 lv1 a:0x2F11D0-0x2F11E0]. So while a port's bit is set in
//! the virtual UART's shadow, the call sets the pending byte again and
//! raises the plug's bit again.
//!
//! # The kernel's calls
//!
//! The kernel makes the call at two sites. Each passes +0x08 and +0x10 of
//! a thread's record, the logical PPE id and the thread, as for
//! `connect_irq_plug_ext`:
//! - [OFW-4.93 lv2_kernel a:0x80000000000080ec-0x8000000000008100] passes
//!   the plug at +48 of an object. The kernel tests r3 after the call
//!   [OFW-4.93 lv2_kernel a:0x8000000000008104-0x8000000000008108].
//! - [OFW-4.93 lv2_kernel a:0x8000000000008d8c-0x8000000000008d9c] is in
//!   the loop over the set bits of the kernel's external interrupt
//!   dispatch [OFW-4.93 lv2_kernel a:0x8000000000008d48-0x8000000000008da8].
//!   The loop makes the call for a plug that holds no kernel handler, and
//!   does not read r3.

/// The handler of hypercall 30 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndOfInterruptHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `end_of_interrupt_ext` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 30 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const END_OF_INTERRUPT_HANDLERS: &[EndOfInterruptHandler] = &[EndOfInterruptHandler {
    firmware: "4.93",
    handler: 0x2E9710,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn end_of_interrupt_handler(firmware: &str) -> Option<&'static EndOfInterruptHandler> {
    END_OF_INTERRUPT_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `end_of_interrupt_ext` for an r3 that names no live logical
/// PPE of the partition: `LV1_NO_ENTRY` (-6), with r3 marked in the high
/// word [OFW-4.93 lv1 a:0x2E98CC-0x2E98D8].
///
/// As with `connect_irq_plug_ext`, bit 32 marks r3, bit 33 r4 and bit 34
/// r5.
pub const END_OF_INTERRUPT_PPE_ID_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `end_of_interrupt_ext` for an r4 that is not below
/// [`super::interrupt::THREADS_PER_LOGICAL_PPE`], unsigned: -6, with r4
/// marked [OFW-4.93 lv1 a:0x2E99B4-0x2E99C0] [OFW-4.93 lv1 a:0x2E9A18-0x2E9A28].
pub const END_OF_INTERRUPT_THREAD_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `end_of_interrupt_ext` for an r5 above 0xFF, unsigned: -6,
/// with r5 marked [OFW-4.93 lv1 a:0x2E99C4-0x2E99C8] [OFW-4.93 lv1 a:0x2E9A60-0x2E9A70].
pub const END_OF_INTERRUPT_PLUG_R3: u64 = 0x0000_0004_FFFF_FFFA;

/// The r3 of `end_of_interrupt_ext` for a plug whose slot holds no port:
/// `LV1_WRONG_STATE` (-11), with r5 marked
/// [OFW-4.93 lv1 a:0x2DFEB0-0x2DFEC0] [OFW-4.93 lv1 a:0x2E99EC-0x2E99F8]
/// [OFW-4.93 lv1 a:0x2E9A2C-0x2E9A3C].
///
/// A port in a plug's slot that is not live or has no connection gives
/// the same r3. A port that CellGov holds is live, and a port in a slot
/// has a connection; see [`super::irq_plug`].
pub const END_OF_INTERRUPT_WRONG_STATE_R3: u64 = 0x0000_0004_FFFF_FFF5;

/// The r3 of `end_of_interrupt_ext` for a port whose pending byte is 0:
/// -11, zero-extended, with no argument marked
/// [OFW-4.93 lv1 a:0x2DFE34-0x2DFE38] [OFW-4.93 lv1 a:0x2E9A40-0x2E9A5C].
pub const END_OF_INTERRUPT_NOT_PENDING_R3: u64 = 0x0000_0000_FFFF_FFF5;

#[cfg(test)]
#[path = "tests/end_of_interrupt_tests.rs"]
mod tests;
