//! The interrupt mask update of a logical PPE thread:
//! - the handler of `did_update_interrupt_mask` (hypercall 31);
//! - its argument checks and the r3 of each;
//! - what lv1 does for the thread that the call names.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! The handler [OFW-4.93 lv1 a:0x2E9354] reads two inputs and no other
//! register:
//! - r3 is a logical PPE id;
//! - r4 is a hardware thread of that PPE, whose mask doublewords in its
//!   interrupt state bitmap changed; see [`super::interrupt`].
//!
//! The handler returns the r3 that it builds in r29
//! [OFW-4.93 lv1 a:0x2E957C]. It writes no return slot, and the vector's
//! entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0.
//!
//! # The thread that the call names
//!
//! The handler compares the PPE object of r3 with the caller's PPE
//! object, and r4 with the caller's thread byte at +1837 of its lv1
//! context [OFW-4.93 lv1 a:0x2E95FC-0x2E9620]. For the caller's own
//! thread, it returns 0 and does nothing else [OFW-4.93 lv1 a:0x2E9624].
//!
//! For the other thread, the handler does these steps:
//! 1. It takes the lock of the PPE [OFW-4.93 lv1 a:0x2E9640-0x2E9670].
//! 2. It takes the lv1 context of thread r4 [OFW-4.93 lv1 a:0x2E9678-0x2E9688].
//! 3. It gives that context to the wake check [OFW-4.93 lv1 a:0x2E9690].
//!    The check does nothing when the byte at +1825 of the context is 0
//!    [OFW-4.93 lv1 a:0x2C6AFC-0x2C6B18]. Otherwise, it ANDs the status
//!    shadow at +1696 with the mask doublewords that +1736 names
//!    [OFW-4.93 lv1 a:0x2C6B40-0x2C6B70]. A nonzero result clears the
//!    byte and runs the wake list at +1816 [OFW-4.93 lv1 a:0x2C6B74-0x2C6B84].
//! 4. It releases the lock [OFW-4.93 lv1 a:0x2E9694-0x2E96A8].
//! 5. It stores priority 0 at +0x410 of the IIC of that thread
//!    [OFW-4.93 lv1 a:0x2E96B4-0x2E96C4] [OFW-4.93 lv1 a:0x2C2D24-0x2C2D44].
//!    That is the thread's interrupt generation port, so the store sends
//!    an interprocessor interrupt to the thread [CBEA p:261 s:21.3.2].
//! 6. It returns 0 [OFW-4.93 lv1 a:0x2E96C8-0x2E96D0] [OFW-4.93 lv1 a:0x2E9624].
//!
//! # The external interrupt
//!
//! On each hypercall's return, lv1's vector ANDs the caller's status
//! shadow at +1696 of its lv1 context with the mask doublewords that
//! +1736 names [OFW-4.93 lv1 a:0x0FC4-0x1010]. For a nonzero result, the
//! function [OFW-4.93 lv1 a:0x18E0] raises the external interrupt
//! [OFW-4.93 lv1 a:0x101C]:
//! - With `MSR[EE]` set in the caller's saved MSR, it returns to the
//!   caller's external interrupt vector [OFW-4.93 lv1 a:0x18E0-0x1918].
//! - Otherwise, it sets bit 0x800 of the LPCR [OFW-4.93 lv1 a:0x191C-0x1928].
//!
//! So a mask change raises the external interrupt only for a bit that is
//! set in the status shadow.
//!
//! # The kernel's calls
//!
//! The kernel makes the call at three sites. Each site passes +0x08 and
//! +0x10 of a thread's record, the logical PPE id and the thread
//! [OFW-4.93 lv2_kernel a:0x8000000000007fe0-0x8000000000007fec]
//! [OFW-4.93 lv2_kernel a:0x8000000000008070-0x800000000000807c]
//! [OFW-4.93 lv2_kernel a:0x8000000000008e48-0x8000000000008e54].
//!
//! The first two sites are in the kernel's mask update
//! [OFW-4.93 lv2_kernel a:0x8000000000007e94]. It compares its record
//! with SPRG0 [OFW-4.93 lv2_kernel a:0x8000000000007ed0-0x8000000000007ee4],
//! and goes on in these cases:
//! - SPRG0 holds the record's address;
//! - the halfword at +0xC0 of the record is 3.
//!
//! It then makes the call only for a halfword of 1 to 3, and only when a
//! mask doubleword gains a bit [OFW-4.93 lv2_kernel a:0x8000000000007f18-0x800000000000806c].

/// The handler of hypercall 31 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptMaskHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `did_update_interrupt_mask` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 31 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const INTERRUPT_MASK_HANDLERS: &[InterruptMaskHandler] = &[InterruptMaskHandler {
    firmware: "4.93",
    handler: 0x2E9354,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn interrupt_mask_handler(firmware: &str) -> Option<&'static InterruptMaskHandler> {
    INTERRUPT_MASK_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `did_update_interrupt_mask` for an r3 that names no live
/// logical PPE of the partition: `LV1_NO_ENTRY` (-6), with r3 marked in
/// the high word [OFW-4.93 lv1 a:0x2E9508-0x2E9514].
///
/// As with `allocate_memory`, bit 32 marks r3 and bit 33 r4. The handler
/// checks the arguments in this order, and the first failure is the
/// answer:
/// 1. the logical PPE id in r3: the object's class word at +48 must be
///    the class that the TOC slot at r2 - 6984 holds, and its state word
///    at +188 must be 1 [OFW-4.93 lv1 a:0x2E94F8-0x2E9504]
///    [OFW-4.93 lv1 a:0x2E95E0-0x2E95E8];
/// 2. the thread in r4, [`INTERRUPT_MASK_THREAD_R3`].
pub const INTERRUPT_MASK_PPE_ID_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `did_update_interrupt_mask` for an r4 that is not below
/// [`super::interrupt::THREADS_PER_LOGICAL_PPE`], unsigned: -6, with r4
/// marked [OFW-4.93 lv1 a:0x2E95EC-0x2E95F8] [OFW-4.93 lv1 a:0x2E962C-0x2E9638].
pub const INTERRUPT_MASK_THREAD_R3: u64 = 0x0000_0002_FFFF_FFFA;

#[cfg(test)]
#[path = "tests/interrupt_mask_tests.rs"]
mod tests;
