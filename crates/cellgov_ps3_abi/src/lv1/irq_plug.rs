//! The IRQ plug of a logical PPE thread:
//! - the handler of `connect_irq_plug_ext` (hypercall 12);
//! - its argument checks and the r3 of each;
//! - the plug table that lv1 keeps for each thread;
//! - what lv1 writes when it connects a port to a plug.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. No byte of lv1 is committed; these facts
//! are.
//!
//! The handler [OFW-4.93 lv1 a:0x2E9DA8] reads five inputs and no other
//! register:
//! - r3 is a logical PPE id;
//! - r4 is a hardware thread of that PPE;
//! - r5 is a plug number of that thread;
//! - r6 is an event receive port id;
//! - r7 must be 0.
//!
//! The handler returns the r3 that it builds in r31
//! [OFW-4.93 lv1 a:0x2E9FE8]. It writes no return slot, and the vector's
//! entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0.
//!
//! lv1 does not compare r4 with the calling thread. Linux
//! `arch/powerpc/platforms/ps3/interrupt.c`, `ps3_irq_plug_setup`, makes
//! the call in this form, with an outlet in r6.
//!
//! # The plug table
//!
//! The logical PPE constructor gives each hardware thread a 4120-byte
//! plug table, and keeps its address at +144 + 8 * thread of the PPE
//! object [OFW-4.93 lv1 a:0x2E10E8-0x2E1134]. For each thread it takes
//! the thread's lv1 context as `configure_irq_state_bitmap` does; see
//! [`super::interrupt`]. The table init [OFW-4.93 lv1 a:0x2DF764] sets
//! these fields:
//! - +8 holds the thread's lv1 context [OFW-4.93 lv1 a:0x2DF7A4].
//! - +16 + 8 * plug holds the port connected to the plug, 0 for none
//!   [OFW-4.93 lv1 a:0x2DF7BC].
//! - +2064 + 8 * plug holds an object reference, 0 at first
//!   [OFW-4.93 lv1 a:0x2DF7C0-0x2DF7D8].
//! - +4112 is 1 [OFW-4.93 lv1 a:0x2DF7E8].
//!
//! So a thread has [`IRQ_PLUGS_PER_THREAD`] plugs.
//!
//! # The connect step
//!
//! The handler gives the function [OFW-4.93 lv1 a:0x2E0B8C] the PPE
//! object, the thread, r5, the port object and 0
//! [OFW-4.93 lv1 a:0x2EA224-0x2EA234]. That function gives the connect
//! step [OFW-4.93 lv1 a:0x2DF968] the port, the thread's plug table, the
//! plug and 0. The connect step does these steps:
//! 1. It returns [`IRQ_PLUG_PORT_R3`] for a port whose byte at +148 is 0
//!    [OFW-4.93 lv1 a:0x2DFA0C-0x2DFA1C].
//! 2. It returns [`IRQ_PLUG_PORT_CONNECTED_R3`] for a port that holds a
//!    connection at +128, and changes nothing
//!    [OFW-4.93 lv1 a:0x2DFA20-0x2DFA28].
//! 3. It stores the plug table at +128 of the port, and the plug at +144
//!    [OFW-4.93 lv1 a:0x2DFA34] [OFW-4.93 lv1 a:0x2DFA3C].
//! 4. It returns a status that the handler maps to `LV1_INTERNAL_ERROR`,
//!    zero-extended, for a table whose byte at +4112 is 0
//!    [OFW-4.93 lv1 a:0x2DFA2C-0x2DFA44] [OFW-4.93 lv1 a:0x2EA264-0x2EA26C].
//! 5. It returns [`IRQ_PLUG_IN_USE_R3`] for a plug whose slot holds a
//!    port [OFW-4.93 lv1 a:0x2DFA48-0x2DFA64].
//! 6. It stores the port in the plug's slot [OFW-4.93 lv1 a:0x2DFA6C].
//! 7. It stores the last argument, 0, at +2064 + 8 * plug, and releases
//!    the object that the slot held [OFW-4.93 lv1 a:0x2DFA70-0x2DFA90].
//! 8. It gives the thread's lv1 context and the plug to one of two
//!    functions [OFW-4.93 lv1 a:0x2DFA94-0x2DFAAC]
//!    [OFW-4.93 lv1 a:0x2DFB78-0x2DFB84]:
//!    - For a port whose pending byte at +149 is set, the function
//!      [OFW-4.93 lv1 a:0x2C65B8] raises the plug's bit.
//!    - Otherwise, the function [OFW-4.93 lv1 a:0x2C3794] clears the
//!      plug's bit. See [`plug_status_bit`].
//! 9. It calls each entry of the list at +96 of the port
//!    [OFW-4.93 lv1 a:0x2DFAF8-0x2DFB4C]. The port constructor empties
//!    the list [OFW-4.93 lv1 a:0x2DF658].
//! 10. It returns 0 [OFW-4.93 lv1 a:0x2DFB50].
//!
//! A failure at step 4 or 5 clears +128 of the port again, and leaves the
//! plug at +144 [OFW-4.93 lv1 a:0x2DFC0C-0x2DFC10].
//!
//! # The kernel's call
//!
//! The kernel's connect step [OFW-4.93 lv2_kernel a:0x8000000000008728]
//! makes the call [OFW-4.93 lv2_kernel a:0x80000000000087f8] with these
//! inputs:
//! - r3 and r4 are +0x08 and +0x10 of the thread's record, the logical
//!   PPE id and the thread [OFW-4.93 lv2_kernel a:0x80000000000087f0-0x80000000000087f4].
//! - r5 is the first free plug of a 256-bit map at +224 of the record,
//!   most significant bit first
//!   [OFW-4.93 lv2_kernel a:0x8000000000007ab4-0x8000000000007b1c]. The
//!   kernel makes no call when the map holds no free plug
//!   [OFW-4.93 lv2_kernel a:0x8000000000008790-0x80000000000087a8].
//! - r6 is the port id that the kernel's caller passes.
//! - r7 is 0 [OFW-4.93 lv2_kernel a:0x800000000000879c].

/// The handler of hypercall 12 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrqPlugHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `connect_irq_plug_ext` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 12 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const IRQ_PLUG_HANDLERS: &[IrqPlugHandler] = &[IrqPlugHandler {
    firmware: "4.93",
    handler: 0x2E9DA8,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn irq_plug_handler(firmware: &str) -> Option<&'static IrqPlugHandler> {
    IRQ_PLUG_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The plugs of one hardware thread. The handler refuses an r5 above
/// 0xFF, unsigned [OFW-4.93 lv1 a:0x2EA074-0x2EA078], and the plug table
/// holds a slot for each plug [OFW-4.93 lv1 a:0x2DF7C0-0x2DF7D8].
pub const IRQ_PLUGS_PER_THREAD: u64 = 256;

/// The status doubleword that holds the bit of `plug`, and the mask of
/// that bit.
///
/// The function [OFW-4.93 lv1 a:0x2C3794] takes doubleword plug / 64 and
/// bit 63 - plug % 64, with bit 0 the least significant
/// [OFW-4.93 lv1 a:0x2C3794-0x2C37BC]. So plug 0 is the most significant
/// bit of the first doubleword.
///
/// The function clears the bit in the thread's status shadow at +1696 of
/// its lv1 context. It then stores the shadow doubleword in the status
/// doubleword that +1728 of the context names
/// [OFW-4.93 lv1 a:0x2C37C0-0x2C37D8]. That is the registered bitmap of
/// the thread, or lv1's own doublewords when the thread registered none
/// [OFW-4.93 lv1 a:0x2C3F88] [OFW-4.93 lv1 a:0x2C4134].
pub const fn plug_status_bit(plug: u8) -> (u64, u64) {
    let plug = plug as u64;
    (plug / 64, 1 << (63 - plug % 64))
}

/// The r3 of `connect_irq_plug_ext` for an r3 that names no live logical
/// PPE of the partition: `LV1_NO_ENTRY` (-6), with r3 marked in the high
/// word [OFW-4.93 lv1 a:0x2E9F74-0x2E9F80].
///
/// As with `allocate_memory`, bit 32 marks r3, bit 33 r4, bit 34 r5,
/// bit 35 r6 and bit 36 r7. The handler checks the arguments in this
/// order, and the first failure is the answer:
/// 1. the logical PPE id in r3: the object's class and its state word at
///    +188, which must be 1 [OFW-4.93 lv1 a:0x2E9F64-0x2E9F70]
///    [OFW-4.93 lv1 a:0x2EA058-0x2EA060];
/// 2. the thread in r4, [`IRQ_PLUG_THREAD_R3`];
/// 3. the plug in r5, [`IRQ_PLUG_PLUG_R3`];
/// 4. r7, [`IRQ_PLUG_R7_R3`];
/// 5. the port in r6, [`IRQ_PLUG_PORT_R3`];
/// 6. the port's connection, [`IRQ_PLUG_PORT_CONNECTED_R3`];
/// 7. the plug's slot, [`IRQ_PLUG_IN_USE_R3`].
pub const IRQ_PLUG_PPE_ID_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `connect_irq_plug_ext` for an r4 that is not below
/// [`super::interrupt::THREADS_PER_LOGICAL_PPE`], unsigned: -6, with r4
/// marked [OFW-4.93 lv1 a:0x2EA064-0x2EA070] [OFW-4.93 lv1 a:0x2EA348-0x2EA354].
pub const IRQ_PLUG_THREAD_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `connect_irq_plug_ext` for an r5 that is not below
/// [`IRQ_PLUGS_PER_THREAD`]: -6, with r5 marked
/// [OFW-4.93 lv1 a:0x2EA334-0x2EA340].
pub const IRQ_PLUG_PLUG_R3: u64 = 0x0000_0004_FFFF_FFFA;

/// The r3 of `connect_irq_plug_ext` for a nonzero r7: -6, with r7 marked
/// [OFW-4.93 lv1 a:0x2EA07C-0x2EA080] [OFW-4.93 lv1 a:0x2EA320-0x2EA32C].
pub const IRQ_PLUG_R7_R3: u64 = 0x0000_0010_FFFF_FFFA;

/// The r3 of `connect_irq_plug_ext` for an r6 that names no event receive
/// port: -6, with r6 marked [OFW-4.93 lv1 a:0x2EA19C-0x2EA1A8].
///
/// The handler returns this r3 in these cases:
/// - The object table holds no object at r6, or the object's class word
///   at +48 is not the class that the TOC slot at r2 - 7072 holds
///   [OFW-4.93 lv1 a:0x2EA18C-0x2EA198].
/// - The port's byte at +148 is 0 [OFW-4.93 lv1 a:0x2EA1F8]
///   [OFW-4.93 lv1 a:0x2EA21C-0x2EA220]. The connect step checks the byte
///   again, and the handler maps that status to this r3
///   [OFW-4.93 lv1 a:0x2EA2A0-0x2EA2AC].
pub const IRQ_PLUG_PORT_R3: u64 = 0x0000_0008_FFFF_FFFA;

/// The r3 of `connect_irq_plug_ext` for a port that is already connected:
/// `LV1_WRONG_STATE` (-11), with r6 marked
/// [OFW-4.93 lv1 a:0x2DFBB8] [OFW-4.93 lv1 a:0x2EA2B0-0x2EA2CC].
pub const IRQ_PLUG_PORT_CONNECTED_R3: u64 = 0x0000_0008_FFFF_FFF5;

/// The r3 of `connect_irq_plug_ext` for a plug whose slot already holds a
/// port: -11, with r5 marked [OFW-4.93 lv1 a:0x2EA238-0x2EA244]
/// [OFW-4.93 lv1 a:0x2EA2D4-0x2EA2E0].
pub const IRQ_PLUG_IN_USE_R3: u64 = 0x0000_0004_FFFF_FFF5;

#[cfg(test)]
#[path = "tests/irq_plug_tests.rs"]
mod tests;
